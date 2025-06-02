# NAS Boot Manager

A Rust-based solution for automatically managing NAS power state based on client activity. Replaces the original Python script running on AsusWRT Merlin routers with a more reliable client-server architecture.

## Overview

This project consists of two components:

1. **NAS Boot Client** - A Windows service that runs on PCs and monitors actual user activity (not just network presence)
2. **NAS Boot Server** - A service that runs on the NAS and manages automatic shutdown based on client heartbeats

## Why This Solution?

The original Python script running on AsusWRT Merlin routers had issues with Windows 11 sleep modes causing false positives for PC activity. This solution:

- Detects actual user activity (keyboard/mouse input) rather than just network presence
- Works reliably with Windows 11 sleep/wake patterns
- Supports Wake-on-LAN over VPN connections
- Provides configurable shutdown delays and conditions

## Components

### NAS Boot Client (Windows Service)

Monitors user activity on Windows PCs and:

- Sends Wake-on-LAN packets when user becomes active
- Sends periodic heartbeats to the NAS server while user is active
- Considers user active based on recent keyboard/mouse input

### NAS Boot Server (NAS Service)

Runs on the NAS and:

- Receives heartbeats from active clients
- Manages automatic shutdown after configurable delay
- Respects keepalive file and backup process conditions

## Installation

### Client (Windows PC)

#### Option 1: Using MSI Installer (Recommended)

1. Download the latest MSI installer from the releases page
2. Run the installer and follow the on-screen instructions
3. Configure the client by editing the configuration file at:
   - `C:\Users\<username>\.config\nas-boot\nas-boot-client-config.yaml`

#### Option 2: Manual Installation

1. Install `cargo wix`:

   ```bash
   cargo install cargo-wix
   ```

2. Build the client:

   ```bash
   cd nas-boot-client
   cargo build --release
   ```

3. Generate default configuration:

   ```bash
   nas-boot-client.exe generate-config
   ```

4. Edit configuration at `%USERPROFILE%\.config\nas-boot\nas-boot-client-config.yaml`:

   ```yaml
   nas_mac: "00:08:9B:DB:EF:9A"
   nas_ip: "192.168.42.2"
   router_ip: "192.168.42.1"
   heartbeat_url: "http://192.168.42.2:8080/cgi-bin/"
   check_interval_secs: 30
   idle_threshold_mins: 5
   heartbeat_timeout_secs: 5
   ```

5. Install service (run as Administrator):

   ```bash
   nas-boot-client.exe install
   ```

6. Start service:

   ```bash
   sc start NASBootClient
   ```

### Building the Windows Installer

To build an MSI installer for easier distribution:

1. Install the WiX Toolset (v3.11 or later) from <https://wixtoolset.org/>
2. Install the cargo-wix subcommand:

   ```bash
   cargo install cargo-wix
   ```

3. Navigate to the nas-boot-client directory:

   ```bash
   cd nas-boot-client
   ```

4. Build the MSI installer:

   ```bash
   cargo wix
   ```

5. The MSI installer will be created in the `target\wix` directory

### Server (NAS - QNAP)

1. Cross-compile the server for your NAS (see cross-compilation section below)

2. Copy the binary to your QNAP:

   ```bash
   scp target/x86_64-unknown-linux-gnu/release/nas-boot-server admin@your-nas-ip:/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/
   ```

3. SSH into your QNAP and make the binary executable:

   ```bash
   chmod +x /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/nas-boot-server
   ```

4. Generate default configuration:

   ```bash
   /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/nas-boot-server generate-config
   ```

5. Edit configuration at `/share/CACHEDEV1_DATA/.config/nas-boot/nas-boot-server-config.yaml`:

   ```yaml
   bind_address: "0.0.0.0:8080"
   shutdown_delay_mins: 10
   keepalive_file: "/share/Public/keepalive.txt"
   backup_process_pattern: "python /share/CACHEDEV1_DATA/.qpkg/AzureStorage/bin/engine.pyc backup"
   heartbeat_timeout_mins: 2
   check_interval_secs: 60
   ```

6. Install as a service using QNAP's autorun system:

   **Step 1: Enable autorun in QNAP settings**
   - Log into QNAP web interface
   - Go to Control Panel → System → Hardware
   - Enable "Run user defined processes during startup (autorun.sh)"

   **Step 2: Create the autorun script**

   ```bash
   # Create the autorun scripts directory
   mkdir -p /share/CACHEDEV1_DATA/.system/autorun/scripts

   # Create the service startup script
   cat > /share/CACHEDEV1_DATA/.system/autorun/scripts/010-nas-boot-server.sh << 'EOF'
   #!/bin/bash

   DAEMON_NAME="nas-boot-server"
   DAEMON_PATH="/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/nas-boot-server"
   PID_FILE="/var/run/${DAEMON_NAME}.pid"
   LOG_FILE="/var/log/${DAEMON_NAME}.log"

   echo "Starting $DAEMON_NAME..."
   
   # Check if already running
   if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
       echo "$DAEMON_NAME is already running (PID: $(cat "$PID_FILE"))"
       exit 0
   fi
   
   # Check if binary exists
   if [ ! -f "$DAEMON_PATH" ]; then
       echo "Error: $DAEMON_PATH not found"
       exit 1
   fi
   
   # Generate config if it doesn't exist
   "$DAEMON_PATH" generate-config 2>/dev/null || true
   
   # Start the daemon
   "$DAEMON_PATH" run > "$LOG_FILE" 2>&1 &
   echo $! > "$PID_FILE"
   
   # Detach from terminal
   disown
   
   # Wait and verify startup
   sleep 2
   if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
       echo "$DAEMON_NAME started successfully (PID: $(cat "$PID_FILE"))"
   else
       echo "$DAEMON_NAME failed to start"
       rm -f "$PID_FILE"
       exit 1
   fi
   EOF

   # Make it executable
   chmod +x /share/CACHEDEV1_DATA/.system/autorun/scripts/010-nas-boot-server.sh
   ```

   **Step 3: Create service management script**

   ```bash
   cat > /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh << 'EOF'
   #!/bin/bash

   DAEMON_NAME="nas-boot-server"
   DAEMON_PATH="/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/nas-boot-server"
   PID_FILE="/var/run/${DAEMON_NAME}.pid"
   LOG_FILE="/var/log/${DAEMON_NAME}.log"

   start() {
       echo "Starting $DAEMON_NAME..."
       if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
           echo "$DAEMON_NAME is already running (PID: $(cat "$PID_FILE"))"
           return 1
       fi
       
       # Check if binary exists
       if [ ! -f "$DAEMON_PATH" ]; then
           echo "Error: $DAEMON_PATH not found"
           return 1
       fi
       
       # Start daemon in background
       "$DAEMON_PATH" run > "$LOG_FILE" 2>&1 &
       echo $! > "$PID_FILE"
       
       # Detach from terminal
       disown
       
       sleep 2
       if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
           echo "$DAEMON_NAME started (PID: $(cat "$PID_FILE"))"
       else
           echo "$DAEMON_NAME failed to start"
           rm -f "$PID_FILE"
           return 1
       fi
   }

   stop() {
       echo "Stopping $DAEMON_NAME..."
       if [ -f "$PID_FILE" ]; then
           PID=$(cat "$PID_FILE")
           if kill -0 "$PID" 2>/dev/null; then
               kill "$PID"
               # Wait for graceful shutdown
               for i in {1..10}; do
                   if ! kill -0 "$PID" 2>/dev/null; then
                       break
                   fi
                   sleep 1
               done
               # Force kill if still running
               if kill -0 "$PID" 2>/dev/null; then
                   kill -9 "$PID"
               fi
           fi
           rm -f "$PID_FILE"
           echo "$DAEMON_NAME stopped"
       else
           echo "$DAEMON_NAME is not running"
       fi
   }

   case "$1" in
       start)   start ;;
       stop)    stop ;;
       restart) stop; sleep 2; start ;;
       status)  
           if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
               echo "$DAEMON_NAME is running (PID: $(cat "$PID_FILE"))"
           else
               echo "$DAEMON_NAME is not running"
           fi
           ;;
       logs)    
           if [ -f "$LOG_FILE" ]; then
               tail -f "$LOG_FILE"
           else
               echo "Log file not found: $LOG_FILE"
           fi
           ;;
       *)
           echo "Usage: $0 {start|stop|restart|status|logs}"
           exit 1
           ;;
   esac
   EOF

   chmod +x /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh
   ```

7. Start the service:

   ```bash
   /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh start
   ```

8. Verify installation:

   ```bash
   # Check service status
   /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh status

   # View logs
   /share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh logs

   # Check autorun logs
   tail /var/log/autorun.log
   ```

The service will now automatically start on boot. You can manage it using:

- **Start**: `/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh start`
- **Stop**: `/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh stop`
- **Restart**: `/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh restart`
- **Status**: `/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh status`
- **View logs**: `/share/CACHEDEV1_DATA/.qpkg/nas-boot-server/service.sh logs`

**Note**: The `010-` prefix ensures this script runs early in the boot process. QNAP executes autorun scripts in alphabetical order.
