# psa-xpanel

A fast, lightweight panel for X11, written in Rust. This panel provides window management, system tray functionality, and clock/date display with efficient rendering and low resource usage.

## Features

- **Lightweight**: Minimal resource usage with efficient rendering
- **X11 Integration**: Full compatibility with X11 window manager protocols
- **Window Management**: Shows open windows with icons and titles
- **System Tray**: Supports system tray icons
- **Clock & Date**: Displays current time and date
- **Active Window Highlighting**: Highlights currently active window
- **Hover Effects**: Visual feedback when hovering over window entries
- **Automatic Window Sizing**: Dynamically adjusts window entry sizes based on available space

## Dependencies

- Rust 2021 edition
- X11 libraries and development headers
- Font configuration (default: OpenSans or DejaVuSans)

## Installation

1. Ensure you have Rust installed (latest stable version recommended)
2. Install X11 development libraries:
   ```bash
   # For Debian/Ubuntu:
   sudo apt install libx11-dev libxrandr-dev libxinerama-dev libxcursor-dev libxcomposite-dev libxdamage-dev
   
   # For Fedora/RHEL:
   sudo dnf install libX11-devel libXrandr-devel libXinerama-devel libXcursor-devel libXcomposite-devel libXdamage-devel
   
   # For Arch Linux:
   sudo pacman -S libx11 libxrandr libxinerama libxcursor libxcomposite libxdamage
   ```
3. Clone the repository:
   ```bash
   git clone https://github.com/shklyarik/psa-xpanel.git
   cd psa-xpanel
   ```
4. Build and run:
   ```bash
   cargo run --release
   ```

## Configuration

The panel uses system default fonts and automatically detects available fonts. Currently, it looks for:
- `/usr/share/fonts/TTF/OpenSans-Light.ttf` (primary)
- `/usr/share/fonts/TTF/DejaVuSans.ttf` (fallback)

## Usage

The panel will automatically:
- Position itself at the bottom of the screen
- Show all open windows with their icons
- Display system tray icons
- Show current time and date
- Highlight the active window
- Provide click-to-focus functionality for window entries

## Architecture

The panel is built using:
- `x11rb`: Safe Rust bindings for X11
- `ab_glyph`: High-quality text rendering
- `image`: Image processing for window icons
- `chrono`: Time handling for clock display

## Performance Optimizations

- Efficient rendering with pixmap caching
- Optimized window list retrieval
- Memory-efficient icon processing
- Event-driven updates to minimize CPU usage

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. For major changes, please open an issue first to discuss what you would like to change.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.