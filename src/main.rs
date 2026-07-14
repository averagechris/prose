#![forbid(unsafe_code)]

use clap::Parser;
use prose::cli::{Cli, execute};
use serde::Serialize;

#[derive(Serialize)]
struct Failure<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{error}");
                return;
            }
            let message = error.to_string().replace('\n', " ").trim().to_owned();
            if std::env::args_os().any(|arg| arg == "--json") {
                fail_json("usage_error", message);
            }
            fail_human(message);
        }
    };
    let json = cli.json;
    match execute(cli) {
        Ok(output) if output.json => println!(
            "{}",
            serde_json::to_string(&output.value).expect("output JSON serialization cannot fail")
        ),
        Ok(output) => println!(
            "{}",
            serde_json::to_string_pretty(&output.value)
                .expect("output JSON serialization cannot fail")
        ),
        Err(error) if json => fail_json(error.code(), error.to_string()),
        Err(error) => fail_human(error.to_string()),
    }
}

fn fail_json(code: &str, message: String) -> ! {
    let output = Failure {
        error: ErrorBody { code, message },
    };
    eprintln!(
        "{}",
        serde_json::to_string(&output).expect("error JSON serialization cannot fail")
    );
    std::process::exit(1)
}

fn fail_human(message: String) -> ! {
    eprintln!("prose: {message}");
    std::process::exit(1)
}
