use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use crate::config::Config;
use anyhow::{Context, Result};

pub async fn wake_nas(config: &Config) -> Result<()> {
    let mac_bytes = parse_mac_address(&config.nas_mac)?;

    // Create magic packet
    let mut packet = vec![0xff; 6]; // 6 bytes of 0xFF
    for _ in 0..16 {
        packet.extend_from_slice(&mac_bytes); // MAC address repeated 16 times
    }

    // Try multiple approaches to ensure delivery
    let mut success = false;

    // 1. Try broadcast on all interfaces
    if let Err(e) = send_wol_broadcast(&packet).await {
        log::warn!("Broadcast WOL failed: {}", e);
    } else {
        success = true;
    }

    // 2. Try directed broadcast to subnet
    if let Some(subnet_broadcast) = get_subnet_broadcast(&config.nas_ip) {
        if let Err(e) = send_wol_to_address(&packet, subnet_broadcast).await {
            log::warn!("Subnet broadcast WOL failed: {}", e);
        } else {
            success = true;
        }
    }

    // 3. Try sending directly to last known IP
    if let Ok(ip) = config.nas_ip.parse::<Ipv4Addr>() {
        if let Err(e) = send_wol_to_address(&packet, ip).await {
            log::warn!("Direct IP WOL failed: {}", e);
        } else {
            success = true;
        }
    }

    if !success {
        return Err(anyhow::anyhow!(
            "Failed to send WOL packet through any method"
        ));
    }

    Ok(())
}

async fn send_wol_broadcast(packet: &[u8]) -> Result<()> {
    // Create socket and enable broadcast
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.set_broadcast(true)?;

    // Send to multiple common WOL ports
    for port in &[7, 9] {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)), *port);
        socket
            .send_to(packet, addr)
            .context("Failed to send WOL broadcast")?;
        log::debug!("Sent WOL packet to broadcast address on port {}", port);
    }

    Ok(())
}

async fn send_wol_to_address(packet: &[u8], ip: Ipv4Addr) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;

    // Try multiple ports
    for port in &[7, 9] {
        let addr = SocketAddr::new(IpAddr::V4(ip), *port);
        socket
            .send_to(packet, addr)
            .context("Failed to send WOL packet")?;
        log::debug!("Sent WOL packet to {} on port {}", ip, port);
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
