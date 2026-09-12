fn main() -> anyhow::Result<()> {
    terrarium::run_cli(std::env::args_os())
}
