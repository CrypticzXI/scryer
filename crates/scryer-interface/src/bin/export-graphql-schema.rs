fn main() {
    let sdl = scryer_interface::export_schema_sdl();
    print!("{sdl}");
    if !sdl.ends_with('\n') {
        println!();
    }
}
