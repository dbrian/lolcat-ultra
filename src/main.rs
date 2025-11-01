use std::io::{self, BufReader};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
const ABOUT: &str = "cat with rainbow colors";

struct Args {
    input: Option<std::path::PathBuf>,
    frequency: f64,
    spread: f64,
    force: bool,
}

/// Print text with rainbow colors using `process_input`
fn print_rainbow(text: &str) {
    let config = lolcat_ultra::Config::try_new(0.04, 4.0, true).unwrap();
    let reader = BufReader::new(text.as_bytes());
    let _ = lolcat_ultra::process_input(reader, &config);
}

fn print_help(program_name: &str) {
    let help_text = format!(
        "{ABOUT}\n\
        \n\
        Usage: {program_name} [OPTIONS] [FILE]\n\
        \n\
        Arguments:\n\
        \x20 [FILE]  input file\n\
        \n\
        Options:\n\
        \x20 -f, --frequency <FREQUENCY>  Color change frequency [default: 0.04]\n\
        \x20 -s, --spread <SPREAD>        Rainbow spread [default: 4.0]\n\
        \x20 -F, --force                  Force color even when stdout is not a tty\n\
        \x20 -h, --help                   Print help\n\
        \x20 -v, --version                Print version\n"
    );
    print_rainbow(&help_text);
}

fn print_version() {
    let version_text = format!("lolcat-ultra {VERSION}\nAuthors: {AUTHORS}\n");
    print_rainbow(&version_text);
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args();
    let program_name = args.next().unwrap_or_else(|| "lolcat-ultra".to_string());

    let mut input: Option<std::path::PathBuf> = None;
    let mut frequency = 0.04;
    let mut spread = 4.0;
    let mut force = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help(&program_name);
                std::process::exit(0);
            }
            "-v" | "--version" => {
                print_version();
                std::process::exit(0);
            }
            "-f" | "--frequency" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value for '{arg}'"))?;
                frequency = value.parse().map_err(|_| {
                    format!("invalid value '{value}' for '{arg}': expected a floating point number")
                })?;
            }
            "-s" | "--spread" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value for '{arg}'"))?;
                spread = value.parse().map_err(|_| {
                    format!("invalid value '{value}' for '{arg}': expected a floating point number")
                })?;
            }
            "-F" | "--force" => {
                force = true;
            }
            arg if arg.starts_with('-') => {
                return Err(format!("unknown option: {arg}"));
            }
            _ => {
                if input.is_some() {
                    return Err("unexpected argument: only one input file is allowed".to_string());
                }
                input = Some(std::path::PathBuf::from(arg));
            }
        }
    }

    Ok(Args {
        input,
        frequency,
        spread,
        force,
    })
}

fn main() {
    // Set up terminal cleanup to ensure proper reset on exit
    lolcat_ultra::setup_terminal_cleanup();

    let program_name = std::env::args()
        .next()
        .unwrap_or_else(|| "lolcat-ultra".to_string());

    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{program_name}: {e}");
            std::process::exit(1);
        }
    };

    // Validate and create config
    let config = match lolcat_ultra::Config::try_new(args.frequency, args.spread, args.force) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{program_name}: {e}");
            std::process::exit(1);
        }
    };

    let result = if let Some(path) = args.input {
        match std::fs::File::open(&path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                lolcat_ultra::process_input(reader, &config)
            }
            Err(e) => {
                eprintln!("{program_name}: {}: {e}", path.display());
                std::process::exit(1);
            }
        }
    } else {
        let stdin = io::stdin();
        let reader = stdin.lock();
        lolcat_ultra::process_input(reader, &config)
    };

    if let Err(e) = result {
        eprintln!("{program_name}: {e}");
        std::process::exit(1);
    }
}
