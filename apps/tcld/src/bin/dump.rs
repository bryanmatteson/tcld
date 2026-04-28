use clap::Parser;
use tcld::DumpOptions;
use tcld::cli::DumpArgs;
use tcld::error::Result;

fn main() {
    let args = DumpArgs::parse();
    if let Err(err) = run(args) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(parsed: DumpArgs) -> Result<()> {
    let output = tcld::dump_path_with_options(
        &parsed.input_path,
        DumpOptions {
            show_all_literals: parsed.show_all_literals,
            literal_filter: parsed.literal_filter,
        },
    )?;
    print!("{}", output);
    Ok(())
}
