//! `soundrts-import <from> <to>`: converts the original's game, one of its
//! packages or one of its maps into the files the game reads.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [from, to] = args.as_slice() else {
        eprintln!(
            "Usage: soundrts-import <from> <to>\n\
             <from> is the folder of the original SoundRTS, or one of its mods, campaigns or \
             maps, as a folder, a .zip file or a .txt file. <to> is the folder or file to make."
        );
        return ExitCode::from(2);
    };
    match soundrts_import::import(Path::new(from), Path::new(to)) {
        Ok(report) => {
            for warning in &report.warnings {
                println!("{warning}");
            }
            println!("{} files written to {to}.", report.files);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Not imported: {e}");
            ExitCode::FAILURE
        }
    }
}
