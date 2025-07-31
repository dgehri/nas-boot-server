# TODO

## Future Improvements

- [ ] Consider using event-driven updates instead of polling for GUI state changes
- [x] Implement timeout wrappers for network operations to prevent blocking ✓
- [x] Add HTTP connection pooling to reduce overhead ✓
- [x] Convert blocking UDP socket operations to async ✓
- [x] Add caching for Windows API calls to reduce frequency ✓
- [ ] Add configurable GUI update intervals in the settings
- [ ] Consider using system tray notifications only on state transitions
- [ ] Implement exponential backoff for failed NAS heartbeat attempts

## Completed Optimizations

- [x] Reduced GUI update frequency from 5s to 30s
- [x] Added HTTP client reuse with connection pooling
- [x] Implemented timeout wrappers for all network operations
- [x] Converted WOL UDP operations to async with timeouts
- [x] Added 10-second caching for user activity detection
- [x] Increased default background check interval to 60s
- [x] Added "Open NAS Web Page" and "Open NAS Drive" tray menu items
- [x] Fixed terminal flash when opening URLs using cross-platform `open` crate
- [x] Increased default background check interval to 60s
