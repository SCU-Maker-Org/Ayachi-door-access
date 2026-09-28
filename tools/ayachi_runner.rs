// Made by Han_feng

macro_rules! runner_println {
    ($($arg:tt)*) => {
        print!("[runner] ");
        println!($($arg)*);
    };
}

include!(concat!(env!("OUT_DIR"), "/runner_config.rs"));

const BLOCK_SIZE: u64 = 64 * 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();

    let mut monitor = true;
    let passthrough = args.iter().skip(2).filter_map(|arg| match arg.as_str() {
        "--no-monitor" => {
            monitor = false;
            None
        },
        "--monitor" => None,
        arg => Some(arg),
    }).collect::<Vec<_>>();

    let build_output = args.get(1).expect("[runner] missing build output");
    let output = format!("target/{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    let output_path = std::path::Path::new(&output);
    std::fs::create_dir_all(output_path).expect("[runner] failed to create output directory");

    let elf_path = output_path.join(format!("{}-v{}.elf", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));
    let image_path = output_path.join(format!("{}-v{}.bin", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")));
    let csv_path = output_path.join("memory_partitions.csv");

    // check the metadata of elf file
    let current = std::fs::metadata(&build_output).ok().and_then(|time| time.modified().ok());
    let last = std::fs::metadata(&elf_path).ok().and_then(|time| time.modified().ok());

    if !matches!((current, last), (Some(current), Some(last)) if current <= last) {
        // copy the elf
        runner_println!("copying the elf file...");
        std::fs::copy(&build_output, &elf_path).expect("[runner] failed to copy the elf file");
        runner_println!("copied the elf file");

        // generate the image bin
        runner_println!("generating the image file...");
        let image_build = std::process::Command::new("espflash")
            .args(["save-image", "--chip", "esp32s3"])
            .arg(build_output)
            .arg(&image_path)
            .status().expect("[runner] failed to launch espflash build");

        if !image_build.success() {
            return Err("[runner] espflash build failed".into());
        }

        runner_println!("generated the image file");

        // memory partitions
        let image_size = std::fs::metadata(&image_path).expect("[runner] failed to open the image file").len();

        runner_println!("image size: {} bytes", image_size);

        let mut csv_buffer = vec![
            "# name,type,sub_type,offset,size,flags".to_string(),
            "nvs,data,nvs,0x9000,0x6000,".to_string(),
            "otadata,data,ota,0xf000,0x2000,".to_string(),
            "phy_init,data,phy,0x11000,0x1000,".to_string(),
        ];

        let mut partition_offset = 0x20000;

        let factory_partition = image_size.next_multiple_of(BLOCK_SIZE);
        csv_buffer.push(format!("origin,app,factory,{:#x},{:#x}", partition_offset, factory_partition));
        partition_offset += factory_partition;

        if partition_offset > FLASH_SIZE {
            return Err(format!("[runner] factory partition too large, it ranges [{:#x}, {:#x}]({} bytes), but flash only arrives at {:#x}", partition_offset - factory_partition, partition_offset, factory_partition, FLASH_SIZE).into());
        }

        let mut index = 0;
        for (name, size) in PRESET {
            let partition = size.next_multiple_of(BLOCK_SIZE);
            csv_buffer.push(format!("{},app,ota_{},{:#x},{:#x}", name, index, partition_offset, partition));
            partition_offset += partition;

            if partition_offset > FLASH_SIZE {
                return Err(format!("[runner] preset partition too large, it ranges [{:#x}, {:#x}](preset: {} bytes, requirement: {} bytes), but flash only arrives at {:#x}", partition_offset - partition, partition_offset, size, partition, FLASH_SIZE).into());
            }

            index += 1;
        }

        let left_memory = FLASH_SIZE - partition_offset;
        let left_slots = SLOTS - index;
        let average_size = left_memory / left_slots as u64;
        let average_partition = average_size / BLOCK_SIZE * BLOCK_SIZE;
        if average_partition <= 0 {
            return Err(format!("It is not enough to share the left memory({} bytes) to the left slots({}), each of which can only have {} bytes(less than the size of one block: {} bytes)", left_memory, left_slots, average_size, BLOCK_SIZE).into())
        }

        for i in index..SLOTS {
            csv_buffer.push(format!("app_{},app,ota_{},{:#x},{:#x}", i, i, partition_offset, average_partition));
            partition_offset += average_partition;
        }

        std::fs::write(&csv_path, csv_buffer.join("\n")).expect("[runner] failed to write memory partitions to disk");

        runner_println!("saved memory partitions(used: {} of {}, {} bytes left)", partition_offset, FLASH_SIZE, FLASH_SIZE - partition_offset);
    }

    let flash = std::process::Command::new("espflash")
        .arg("flash")
        .arg(&build_output)
        .args(["--partition-table", csv_path.to_str().unwrap()])
        .args(["--target-app-partition", "origin"])
        .args(["--erase-parts", "otadata"])
        .args(["--monitor"].into_iter().filter(|_| monitor))
        .args(passthrough)
        .status().expect("[runner] failed to launch espflash flash");

    if !flash.success() {
        return Err(format!("[runner] espflash flash failed: {}", flash).into());
    }

    runner_println!("finished flashing");

    Ok(())
}