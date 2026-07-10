//! `modload` — standalone CLI to load + validate a mod (folder or `.zip`)
//! against the host ABI, without spinning up the engine. Handy for CI smoke
//! checks of third-party mods: exits non-zero if the mod fails to load,
//! validate, or register.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: modload <mod-path>");
        return ExitCode::FAILURE;
    };
    match stormlight_modloader::report::describe(Path::new(&path)) {
        Ok(summary) => {
            print!("{summary}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("modload: {err:#}");
            ExitCode::FAILURE
        }
    }
}
