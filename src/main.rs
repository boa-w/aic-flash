mod image;
mod protocol;
mod usb;

use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "aic-flash",
    version,
    about = "Cross-platform flasher for ArtInChip SoCs"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan for connected ArtInChip devices
    Scan,
    /// Show the currently selected project, output directory, and toolchain
    Info {
        /// Optional .img file to parse instead of querying a device
        #[arg(value_name = "IMAGE")]
        image: Option<PathBuf>,
    },
    /// Burn firmware image to device
    Burn {
        /// Path to the firmware image file (.img)
        #[arg(value_name = "IMAGE")]
        image: PathBuf,
        /// Do not reset device after burn
        #[arg(long)]
        no_reset: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Scan => cmd_scan(),
        Commands::Info { image } => cmd_info(image),
        Commands::Burn { image, no_reset } => cmd_burn(image, no_reset),
    }
}

fn cmd_scan() {
    match usb::device::AicDevice::open_first() {
        Ok(mut dev) => {
            println!("Found ArtInChip device!");
            if let Err(e) = dev.show_info() {
                eprintln!("Error reading device info: {}", e);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}

fn cmd_info(image: Option<PathBuf>) {
    if let Some(path) = image {
        // Parse local image file
        match fs::read(&path) {
            Ok(data) => {
                if let Err(e) = image::parser::print_image_info(&data) {
                    eprintln!("Error parsing image: {}", e);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error reading '{}': {}", path.display(), e);
                std::process::exit(1);
            }
        }
    } else {
        // Query device
        match usb::device::AicDevice::open_first() {
            Ok(mut dev) => {
                println!("=== Device Info ===");
                if let Err(e) = dev.show_info() {
                    eprintln!("Error reading device info: {}", e);
                    std::process::exit(1);
                }
                // Also try storage media
                match dev.get_storage_media() {
                    Ok(media) => println!("  Storage media: {}", media),
                    Err(_) => {}
                }
            }
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    }
}

fn cmd_burn(image: PathBuf, no_reset: bool) {
    // 1. Read image file
    let img_data = match fs::read(&image) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error reading '{}': {}", image.display(), e);
            std::process::exit(1);
        }
    };

    // 2. Parse image
    let (header, metas, _payload) = match image::parser::parse_image(&img_data) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error parsing image: {}", e);
            std::process::exit(1);
        }
    };

    println!(
        "Image: {} {} v{} ({} components, {} bytes total)",
        header.platform_str(),
        header.product_str(),
        header.version_str(),
        metas.len(),
        img_data.len()
    );

    // 3. Connect to device
    let mut dev = match usb::device::AicDevice::open_first() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    // 4. Show device info
    if let Err(e) = dev.show_info() {
        eprintln!("Warning: could not read device info: {}", e);
    }

    // 5. Burn!
    if let Err(e) = dev.burn_image(&img_data, &metas, &header) {
        eprintln!("Burn failed: {}", e);
        std::process::exit(1);
    }

    println!("Burn completed successfully!");

    // 6. Reset if requested
    if !no_reset {
        println!("Resetting device...");
        if let Err(e) = dev.reset() {
            eprintln!("Warning: reset failed: {}", e);
        } else {
            println!("Device reset.");
        }
    }
}
