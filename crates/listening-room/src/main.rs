//! `listening-room`: see the crate documentation and README.md.

use std::path::PathBuf;
use std::process::ExitCode;

use listening_room::script::{Kind, NUM_STEPS, Options, run};

const USAGE: &str = "\
Usage: listening-room [options]

Plays the game's sounds through each renderer and asks what you hear.
Use headphones. Answers are saved in listening-room-results.txt.

Options:
  --root <folder>        The game's data folder, as for the game.
  --step <n>             Run only step n, from 1 to 6.
  --renderers <names>    Renderers to compare, separated by commas:
                         current, stereo-plus, binaural. All by default.
  --sofa <file>          A head model (SOFA file) for the binaural renderer.
  --render-to <folder>   Write each sound to a WAV file there and play nothing.
                         The answers file goes there too.
  --seed <n>             Repeat the random order of an earlier session.
  --help                 Print this and stop.";

fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut opts = Options {
        root: None,
        step: None,
        kinds: Kind::built(),
        sofa: None,
        render_to: None,
        seed: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_secs()),
    };
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} needs a value. Run with --help for the options."))
        };
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--root" => opts.root = Some(PathBuf::from(value()?)),
            "--sofa" => opts.sofa = Some(PathBuf::from(value()?)),
            "--render-to" => opts.render_to = Some(PathBuf::from(value()?)),
            "--step" => {
                let v = value()?;
                match v.parse::<usize>() {
                    Ok(n @ 1..=NUM_STEPS) => opts.step = Some(n),
                    _ => return Err(format!("--step takes a number from 1 to {NUM_STEPS}.")),
                }
            }
            "--seed" => {
                opts.seed = value()?
                    .parse()
                    .map_err(|_| "--seed takes a whole number.".to_owned())?;
            }
            "--renderers" => {
                opts.kinds.clear();
                for name in value()?.split(',') {
                    let kind = Kind::parse(name.trim())?;
                    if !opts.kinds.contains(&kind) {
                        opts.kinds.push(kind);
                    }
                }
            }
            _ => {
                return Err(format!(
                    "Unknown option {arg}. Run with --help for the options."
                ));
            }
        }
    }
    if opts.sofa.is_some() && !opts.kinds.contains(&Kind::Binaural) {
        return Err(if Kind::Binaural.is_built() {
            "--sofa is for the binaural renderer, which is not chosen.".to_owned()
        } else {
            "--sofa is for the binaural renderer, which this build does not have. Build with --features binaural.".to_owned()
        });
    }
    Ok(Some(opts))
}

fn main() -> ExitCode {
    let opts = match parse(std::env::args().skip(1)) {
        Ok(Some(opts)) => opts,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    match run(&opts, &mut input, &mut out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
