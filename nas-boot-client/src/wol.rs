use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

use crate::config::Config;
use anyhow::{Context, Result};

pub async fn wake_nas(config: &Config) -> Result<()> {
    let mac_bytes = parse_mac_address(&config.nas_mac)?;

    // Create magic packet
    let mut packet = vec![0xff; 6]; // 6 bytes of 0xFF
    for _ in 0..16 {
        packet.extend_from_slice(&mac_bytes); // MAC address repeated 16 times
    }

    // Try multiple approaches with timeouts to prevent blocking
    let mut success = false;

    // 1. Try broadcast on all interfaces with timeout
    match timeout(Duration::from_secs(2), send_wol_broadcast(&packet)).await {
        Ok(Ok(())) => success = true,
        Ok(Err(e)) => log::warn!("Broadcast WOL failed: {e}"),
        Err(_) => log::warn!("Broadcast WOL timed out"),
    }

    // 2. Try directed broadcast to subnet with timeout
    if let Some(subnet_broadcast) = get_subnet_broadcast(&config.nas_ip) {
        match timeout(Duration::from_secs(2), send_wol_to_address(&packet, subnet_broadcast)).await {
            Ok(Ok(())) => success = true,
            Ok(Err(e)) => log::warn!("Subnet broadcast WOL failed: {e}"),
            Err(_) => log::warn!("Subnet broadcast WOL timed out"),
        }
    }

    // 3. Try sending directly to last known IP with timeout
    if let Ok(ip) = config.nas_ip.parse::<Ipv4Addr>() {
        match timeout(Duration::from_secs(2), send_wol_to_address(&packet, ip)).await {
            Ok(Ok(())) => success = true,
            Ok(Err(e)) => log::warn!("Direct IP WOL failed: {e}"),
            Err(_) => log::warn!("Direct IP WOL timed out"),
        }
    }

    if !success {
        log::warn!("All WOL methods failed or timed out, but continuing...");
        // Don't return error - WOL failures shouldn't crash the app
    }

    Ok(())
}

async fn send_wol_broadcast(packet: &[u8]) -> Result<()> {
    // Create async socket and enable broadcast
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;

    // Send to multiple common WOL ports
    for port in &[7, 9] {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)), *port);
        socket
            .send_to(packet, addr)
            .await
            .context("Failed to send WOL broadcast")?;
        log::debug!("Sent WOL packet to broadcast address on port {port}");
    }

    Ok(())
}

async fn send_wol_to_address(packet: &[u8], ip: Ipv4Addr) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;

    // Try multiple ports
    for port in &[7, 9] {
        let addr = SocketAddr::new(IpAddr::V4(ip), *port);
        socket
            .send_to(packet, addr)
            .await
            .context("Failed to send WOL packet")?;
        log::debug!("Sent WOL packet to {ip} on port {port}");
    }

    Ok(())
}

fn parse_mac_address(mac: &str) -> Result<[u8; 6]> {
    let mac = mac.replace([':', '-'], "");

    if mac.len() != 12 {
        return Err(anyhow::anyhow!("Invalid MAC address length"));
    }

    let mut bytes = [0u8; 6];
    for (i, chunk) in mac.as_bytes().chunks(2).enumerate() {
        let byte_str = std::str::from_utf8(chunk)?;
        bytes[i] = u8::from_str_radix(byte_str, 16).context("Invalid hex in MAC address")?;
    }

    Ok(bytes)
}

fn get_subnet_broadcast(nas_ip: &str) -> Option<Ipv4Addr> {
    // Parse IP address
    let ip = nas_ip.parse::<Ipv4Addr>().ok()?;

    // Assume common subnet masks - ideally this should be configurable
    // For 192.168.x.x, assume /24 subnet
    if ip.octets()[0] == 192 && ip.octets()[1] == 168 {
        Some(Ipv4Addr::new(
            ip.octets()[0],
            ip.octets()[1],
            ip.octets()[2],
            255,
        ))
    } else if ip.octets()[0] == 10 {
        // For 10.x.x.x, assume /24 subnet
        Some(Ipv4Addr::new(
            ip.octets()[0],
            ip.octets()[1],
            ip.octets()[2],
            255,
        ))
    } else {
        None
    }
}
