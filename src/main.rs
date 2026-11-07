use std::process::ExitCode;

fn main() -> ExitCode {
    let args = match audioswitcher::deck::parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match audioswitcher::deck::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("audioswitcher: {e}");
            ExitCode::FAILURE
        }
    }
}
