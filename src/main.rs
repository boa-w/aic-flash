use std::fs;
use std::path::PathBuf;

use aic_flash::image;
use aic_flash::standalone;
use aic_flash::usb;
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
    /// Check local config, USB access, and optional image parsing
    EnvCheck {
        /// Optional .img file to parse during the check
        #[arg(value_name = "IMAGE")]
        image: Option<PathBuf>,
    },
    /// Install platform USB access support (WinUSB INF or Linux udev rule)
    InstallUsbAccess,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Scan => cmd_scan(),
        Commands::Info { image } => cmd_info(image),
        Commands::Burn { image, no_reset } => cmd_burn(image, no_reset),
        Commands::EnvCheck { image } => cmd_env_check(image),
        Commands::InstallUsbAccess => cmd_install_usb_access(),
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
    let options = usb::device::BurnOptions {
        reset_after_burn: !no_reset,
        ..Default::default()
    };
    if let Err(e) = dev.burn_image_with_options(&img_data, &metas, &options, None) {
        eprintln!("Burn failed: {}", e);
        std::process::exit(1);
    }

    println!("Burn completed successfully!");
}

fn cmd_env_check(image: Option<PathBuf>) {
    println!("{}", standalone::environment_report(image.as_deref()));
}

fn cmd_install_usb_access() {
    match standalone::install_driver() {
        Ok(()) => println!("USB access setup completed."),
        Err(e) => {
            eprintln!("USB access setup failed: {}", e);
            std::process::exit(1);
        }
    }
}
