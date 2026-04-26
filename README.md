# TailChatter

A desktop chat application for Tailscale networks. Built with egui for a native GUI experience.

## Prerequisites

- **Linux**: Build from source (see below) or download from GitHub releases
- **Windows**: Download the `.exe` from GitHub Actions artifacts

## Quick Start

### 1. Start the Server

You'll need a running server. Clone and run the server from the rustalk repository:

```bash
git clone https://github.com/cozyydev/rustalk.git
cd rustalk
cargo run --bin rustalk
```

The server listens on `0.0.0.0:42069`.

### 2. Run TailChatter

#### Linux

```bash
# Clone this repo
git clone https://github.com/cozyydev/tailchatter.git
cd tailchatter/tailchatter-egui

# Build
cargo build --release

# Run
./target/release/tailchatter
```

#### Windows

1. Download the `tailchatter-windows-x64` artifact from GitHub Actions
2. Run the `.exe`
3. Enter your details:
   - **Your Handle**: Your nickname
   - **Server IP**: Your Tailscale server IP
   - **Port**: 42069 (or your server's port)

## Recommended Network Model

TailChatter is designed for private use over Tailscale.

**Setup:**
- Run the rustalk server on one machine in your Tailscale network
- Connect clients from other machines using their Tailscale IPs
- No public internet exposure by default

### Getting Your Tailscale IP

On the server machine:

```bash
tailscale ip -4
```

Use that IP when connecting clients.

## Usage

1. Launch TailChatter
2. Enter your nickname (2-24 characters, letters, numbers, `_` or `-`)
3. Enter the server's Tailscale IP address
4. Enter the port (default: 42069)
5. Click "Join Chat" or press Enter

### Sending Messages

- Type your message in the input box at the bottom
- Press **Enter** to send
- Or click the **Send** button

### Online Users

The left sidebar shows all connected users. Your nickname is highlighted in cyan.

## Building from Source

### Linux

```bash
cd tailchatter-egui
cargo build --release
./target/release/tailchatter
```

### Windows (via GitHub Actions)

The Windows build is automated via GitHub Actions:

1. Go to your repository's Actions tab
2. Run the "Build TailChatter for Windows" workflow
3. Download the artifact `tailchatter-windows-x64`

## Project Status

- Desktop GUI client (egui)
- TCP connection to rustalk server
- Real-time message display
- Send messages via Enter or button click
- Online user list sidebar
- Dracula-themed UI
- Native window with title bar
- Linux and Windows builds

## License

MIT