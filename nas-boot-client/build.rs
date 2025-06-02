use {
    std::{env, io},
    winresource::WindowsResource,
};

fn main() -> io::Result<()> {
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        WindowsResource::new()
            .set_icon("assets/nas_black.ico")
            .set_icon("assets/nas_green.ico")
            .set_icon("assets/nas_red.ico")
            .set_resource_file("assets/resources.rc")
            .compile()?;
    }
    Ok(())
}
