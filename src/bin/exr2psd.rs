use std::env;
use std::path::{Path, PathBuf};
use std::fs::File;

fn print_usage() {
    eprintln!("Usage: exr2psd <input.exr> [output.psd]");
}

fn default_out_path(input: &Path) -> PathBuf {
    let mut out = input.to_path_buf();
    out.set_extension("psd");
    out
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        print_usage();
        std::process::exit(2);
    }

    let input_path = Path::new(&args[1]);
    let output_path = if args.len() == 3 { PathBuf::from(&args[2]) } else { default_out_path(input_path) };

    if !input_path.exists() {
        eprintln!("Error: input file not found: {}", input_path.display());
        std::process::exit(2);
    }

    // Wczytaj warstwy z EXR (szkielet)
    let (layers, composite) = match exr_layers::read_layers(input_path) {
        Ok(ok) => ok,
        Err(e) => {
            eprintln!("Error reading EXR: {}", e);
            std::process::exit(3);
        }
    };

    // Zapisz PSD (szkielet)
    let file = match File::create(&output_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error creating output file '{}': {}", output_path.display(), e);
            std::process::exit(4);
        }
    };

    if let Err(e) = psd_writer::write_psd(file, &layers, &composite) {
        eprintln!("Error writing PSD: {}", e);
        std::process::exit(4);
    }

    println!("OK: {}", output_path.display());
}


