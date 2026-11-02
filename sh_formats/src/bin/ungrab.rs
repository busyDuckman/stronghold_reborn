// --------------------------------------------------------------------------------------------------------------------
// ungrab: STRONG.DAT back into the source tree STRONG.EXE grab builds it from. A thin shell over sh_formats::grab.
//
//   ungrab STRONG.DAT out/                  every file grab opens, at its DOS path, plus ungrab.txt and ungrab.tsv
//   ungrab --accept-layout MY.DAT out/      a container grab rebuilt from an edited tree, checked entry by entry
// The output folder must be new or empty. Rebuild with the English CD STRONG.EXE: see ungrab.txt in the output.

use clap::Parser;
use sh_formats::grab::{ungrab, Recognised};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "ungrab", about = "Write the source files STRONG.EXE grab rebuilds STRONG.DAT from", version)]
struct Cli {
    /// The archive to take apart.
    dat: PathBuf,
    /// Where the source tree goes; must be new or empty.
    out: PathBuf,
    /// Accept a STRONG.DAT whose md5 is not the shipped one when every entry has the kind and shape grab makes.
    #[arg(long)]
    accept_layout: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let run = || -> sh_formats::error::Result<()> {
        let dat = std::fs::read(&cli.dat).map_err(|source| sh_formats::error::FormatError::Io { path: cli.dat.clone(), source })?;
        let tree = ungrab(&dat, cli.accept_layout)?;
        let written = tree.write_to(&cli.out)?;
        let how = match &tree.recognised {
            Recognised::Shipped => "the shipped English CD archive".to_string(),
            Recognised::ByLayout { md5 } => format!("an archive recognised by layout, md5 {md5}"),
        };
        println!("ungrab: {} from {how}", cli.dat.display());
        println!("ungrab: {written} files written to {} ({} pictures, {} sources, ungrab.txt, ungrab.tsv)", cli.out.display(), tree.pictures, tree.files.len());
        Ok(())
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
