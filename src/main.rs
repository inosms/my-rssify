use clap::Parser;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = rssify::Cli::parse();
    rssify::run(cli)
}
