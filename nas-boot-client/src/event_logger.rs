use log::{Level, LevelFilter, Metadata, Record, SetLoggerError};
use std::ptr;
use std::sync::Once;
use winapi::shared::minwindef::WORD;
use winapi::um::winnt::{LPCWSTR, HANDLE};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::{
    DeregisterEventSource, RegisterEventSourceW, ReportEventW,
};
// These constants are actually in winnt.h, not winbase.h
use winapi::um::winnt::{
    EVENTLOG_ERROR_TYPE, EVENTLOG_WARNING_TYPE, EVENTLOG_INFORMATION_TYPE,
    EVENTLOG_SUCCESS,
};
use windows_service::service::{
    ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::ServiceStatusHandle;

static INIT: Once = Once::new();
static mut EVENT_SOURCE: Option<HANDLE> = None;
pub static mut STATUS_HANDLE: Option<ServiceStatusHandle> = None;

pub struct EventLogger;

fn to_wide_string(s: &str) -> Vec<u16> {
    let mut result: Vec<u16> = s.encode_utf16().collect();
    result.push(0); // null terminator
    result
}

impl EventLogger {
    pub fn init(status_handle: Option<ServiceStatusHandle>) -> Result<(), SetLoggerError> {
        unsafe {
            INIT.call_once(|| {
                // Store the service status handle for later use
                STATUS_HANDLE = status_handle;
                
                // Register the event source
                let source_name = to_wide_string("NASBootClient");
                let handle = RegisterEventSourceW(
                    ptr::null_mut(), 
                    source_name.as_ptr() as LPCWSTR
                );
                
                if !handle.is_null() {
                    EVENT_SOURCE = Some(handle);
                    // Log a startup event to the Windows Event Log
                    Self::log_to_event_log(
                        EVENTLOG_INFORMATION_TYPE,
                        "NAS Boot Client service started."
                    );
                } else {
                    let error = GetLastError();
                    eprintln!("Failed to register Windows Event Log source. Error code: {}", error);
                }
            });
        }

        log::set_boxed_logger(Box::new(EventLogger))?;
        log::set_max_level(LevelFilter::Info);
        Ok(())
    }

    // Helper to write to the Windows Event Log directly
    fn log_to_event_log(event_type: WORD, message: &str) {
        unsafe {
            if let Some(source) = EVENT_SOURCE {
                let wide_message = to_wide_string(message);
                let mut strings_ptr = [wide_message.as_ptr()];
                
                ReportEventW(
                    source,                   // event log handle
                    event_type,               // event type
                    0,                        // category
                    1,                        // event ID (using 1 for general messages)
                    ptr::null_mut(),          // user SID
                    1,                        // number of strings
                    0,                        // no binary data
                    strings_ptr.as_mut_ptr(), // array of strings
                    ptr::null_mut()           // no binary data
                );
            }
        }
    }
    
    pub fn shutdown() {
        unsafe {
            if let Some(source) = EVENT_SOURCE {
                // Log a shutdown event
                Self::log_to_event_log(
                    EVENTLOG_INFORMATION_TYPE,
                    "NAS Boot Client service stopped."
                );
                
                // Close the event source
                DeregisterEventSource(source);
                EVENT_SOURCE = None;
            }
        }
    }
}

impl Drop for EventLogger {
    fn drop(&mut self) {
        // In case the explicit shutdown wasn't called
        EventLogger::shutdown();
    }
}

impl log::Log for EventLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        // Write to stderr for debugging and log files
        eprintln!(
            "{} - {}: {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            record.level(),
            record.args()
        );

        // Map log levels to Windows event types
        let event_type = match record.level() {
            Level::Error => EVENTLOG_ERROR_TYPE,
            Level::Warn => EVENTLOG_WARNING_TYPE,
            Level::Info => EVENTLOG_INFORMATION_TYPE,
            Level::Debug => EVENTLOG_SUCCESS,
            Level::Trace => EVENTLOG_SUCCESS,
        };
        
        // Log to Windows Event Log
        EventLogger::log_to_event_log(event_type, &format!("{}", record.args()));

        // If we have a service handle, update service status for errors
        unsafe {
            if let Some(ref handle) = STATUS_HANDLE {
                if record.level() <= Level::Warn {
                    let exit_code = match record.level() {
                        Level::Error => ServiceExitCode::ServiceSpecific(1),
                        Level::Warn => ServiceExitCode::ServiceSpecific(2),
                        _ => ServiceExitCode::Win32(0),
                    };

                    // Update service status with the message
                    let _ = handle.set_service_status(ServiceStatus {
                        service_type: ServiceType::OWN_PROCESS,
                        current_state: ServiceState::Running,
                        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                        exit_code,
                        checkpoint: 0,
                        wait_hint: std::time::Duration::default(),
                        process_id: None,
                    });
                }
            }
        }
    }

    fn flush(&self) {}
}
