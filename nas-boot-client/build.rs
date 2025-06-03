use {std::io, winresource::WindowsResource};

fn main() -> io::Result<()> {
    if cfg!(windows) {
        let base_icon = image::open("assets/nas_black.ico").unwrap().to_rgba8();

        let colors = [
            ("green", (6, 156, 76)),
            ("red", (212, 44, 15)),
            ("yellow", (224, 189, 11)),
            ("grey", (128, 128, 128)),
        ];

        for (name, color) in colors {
            let mut tinted = image::ImageBuffer::new(base_icon.width(), base_icon.height());

            for (x, y, pixel) in base_icon.enumerate_pixels() {
                let image::Rgba([_r, _g, _b, a]) = *pixel;
                if a > 0 {
                    tinted.put_pixel(x, y, image::Rgba([color.0, color.1, color.2, a]));
                } else {
                    tinted.put_pixel(x, y, *pixel);
                }
            }

            tinted.save(format!("assets/nas_{name}.ico")).unwrap();
        }

        WindowsResource::new()
            .set_icon("assets/nas_black.ico")
            .set_icon("assets/nas_green.ico")
            .set_icon("assets/nas_red.ico")
            .set_icon("assets/nas_yellow.ico")
            .set_resource_file("assets/resources.rc")
            .compile()?;

        // Rerun if the base icon changes
        println!("cargo:rerun-if-changed=assets/nas_black.ico");
        println!("cargo:rerun-if-changed=assets/resources.rc");
    }
    Ok(())
}
