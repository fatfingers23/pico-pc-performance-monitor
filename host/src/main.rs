use icd::GetLedEndpoint;
use poststation_sdk::{StreamListener, connect};

#[tokio::main]
async fn main() -> Result<(), String> {
    println!("Hello, world!");
    //TODO don't leave the unwraps
    let client = connect("localhost:51837").await.unwrap();
    let connected_devices = client.get_devices().await.unwrap();

    let first_connected_device = connected_devices
        .iter()
        .filter(|d| d.is_connected == true)
        .next()
        .unwrap();

    println!("Connected devices: {:?}", connected_devices);
    let status = client
        .proxy_endpoint::<GetLedEndpoint>(first_connected_device.serial, 0, &())
        .await
        .unwrap();
    println!("Status: {:?}", status);

    Ok(())
}
