use dotenv::dotenv;
use env_logger::Env;
use icd::{SetDisplayEndpoint, SysInfo};
use log::{debug, error, info};
use poststation_sdk::connect;
use std::env;
use std::time::Duration;
use sysinfo::System;
use tokio::process::Command;
use tokio::signal;
use tokio::time::{interval, sleep};

#[tokio::main]
async fn main() {
    //Sets up the logger. Can set RUST_LOG=debug as env variable to see more detailed logs
    let env = Env::default().filter_or("RUST_LOG", "info");
    env_logger::init_from_env(env);
    //Loads in the .env file variables
    dotenv().ok();

    match spawn_poststation().await {
        Some(mut poststation_process) => {
            //Launch the actual logic of the program in a separate thread
            tokio::spawn(async move {
                //Gives time for poststation to start up. May not be needed
                sleep(Duration::from_millis(500)).await;
                let result = do_work().await;
                if let Err(e) = result {
                    error!("{:?}", e);
                }
            });

            // Set up a signal handler to kill poststation process when the parent exits
            tokio::select! {
                _ = signal::ctrl_c() => {
                    println!("Received Ctrl+C, killing Poststation process...");
                    poststation_process.kill().await.expect("Failed to kill Poststation process");
                }
                status = poststation_process.wait() => {
                    info!("Poststation process exited with status: {:?}", status);
                    info!("If this was unintentional, may either run Poststation manually and remove the env POSTSTATION_LOCATION or stop the other instance of Poststation");
                }
            }
        }
        None => {
            info!(
                "Poststation process did not start, can check above errors for more details. Continuing in case it was launched manually"
            );
            //Launch the actual logic of the program to capture computer usage and display it on the pico
            let result = do_work().await;
            if let Err(e) = result {
                error!("{:?}", e);
            }
        }
    }
}

/// The actual logic of the program to capture computer usage and display it on the pico
async fn do_work() -> Result<(), String> {
    let client = match connect("localhost:51837").await {
        Ok(c) => c,
        Err(e) => {
            error!("{:?}", e);
            return Err(
                "Error connecting to poststation, please make sure it is running".to_string(),
            );
        }
    };

    let connected_devices = match client.get_devices().await {
        Ok(connected_devices) => connected_devices,
        Err(e) => {
            error!("{:?}", e);
            return Err("Error getting connected devices.".to_string());
        }
    };

    let first_connected_device = connected_devices
        .iter()
        .filter(|d| d.is_connected == true)
        .next()
        .unwrap_or_else(|| {
            error!("No connected devices found. Poststation is running, please make sure you have an active device connected");            
            std::process::exit(1);
        });

    info!("First connected device: {:?}", first_connected_device);

    let mut sys = System::new_all();

    let mut message_seq_number = 0;
    let host_name = System::host_name().unwrap_or("".to_string());

    let mut interval = interval(Duration::from_millis(500));

    loop {
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let cpu_uasge = sys.global_cpu_usage();
        let cpu_avg_freq =
            sys.cpus().iter().map(|cpu| cpu.frequency()).sum::<u64>() / sys.cpus().len() as u64;
        //Since we have the full power of the computer we format the cpu frequency here
        let cpu_avg_freq_str = if cpu_avg_freq > 1000 {
            format!("{:.2}GHz", cpu_avg_freq as f64 / 1000.0)
        } else {
            format!("{:.2}MHz", cpu_avg_freq)
        };

        //Does the math here to move fractions to easier types for the pico to work with and display
        let memory_usage = sys.used_memory();
        let memory_usage_mb = (memory_usage as f64 / 1024.0 / 1024.0).round() as u64;
        let total_memory = sys.total_memory();
        let total_memory_mb = (total_memory as f64 / 1024.0 / 1024.0).round() as u64;
        let formatted_to_u8 = cpu_uasge.round() as u8;

        //Scrolls the text using spaces on each message
        let scroll_text = "Poststation.rs".to_string();
        let scroll_length = 18;
        let scroll_position = message_seq_number % (scroll_text.len() + scroll_length);
        let scroll_text = if scroll_position < scroll_length {
            format!("{:width$}", "", width = scroll_length - scroll_position)
                + &scroll_text[..scroll_position.min(scroll_text.len())]
        } else {
            scroll_text[scroll_position - scroll_length..].to_string()
        };

        let sys_info = SysInfo {
            host_name: host_name.as_str(),
            cpu_freq_text: cpu_avg_freq_str.as_str(),
            cpu_usage: formatted_to_u8,
            memory_usage: memory_usage_mb,
            total_memory: total_memory_mb,
            scroll_text: scroll_text.as_str(),
        };

        debug!("SysInfo: {:?}", sys_info);

        let result = client
            .proxy_endpoint::<SetDisplayEndpoint>(
                first_connected_device.serial,
                message_seq_number as u32,
                &sys_info,
            )
            .await;
        message_seq_number += 1;

        if let Err(e) = result {
            error!("{:?}", e);
        }
        interval.tick().await;
    }
}

/// This method spawns poststation in headless mode so we don't have to manually launch it
/// If you do not have the env set the program will still work, just need to manually start poststation
async fn spawn_poststation() -> Option<tokio::process::Child> {
    match env::var("POSTSTATION_LOCATION") {
        Ok(poststation_location) => {
            let process_spawned = Command::new(poststation_location).arg("--headless").spawn();

            match process_spawned {
                Ok(poststation_process) => {
                    debug!(
                        "headless Poststation process running in background with PID: {}",
                        poststation_process.id().unwrap()
                    );

                    Some(poststation_process)
                }
                Err(_) => {
                    error!(
                        "Error starting poststation. Please make sure the path is correct and the file is executable"
                    );
                    None
                }
            }
        }
        Err(_) => {
            info!(
                "Looks like POSTSTATION_LOCATION was not found in your .env file. You can set the location of your poststation by creating a .env and adding it, or just launch poststation manually"
            );
            None
        }
    }
}
