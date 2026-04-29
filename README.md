# TailChatter

A desktop chat application for Tailscale networks. Built with egui for a native GUI experience. Includes a built-in server - you can connect to existing Tailscale nodes or start your own server with your own Tailscale IP.

## Prerequisites

- **Linux & macOS**: Build from source (see below)
- **Windows**: Download the `.exe` from GitHub Releases

## Quick Start

### Run TailChatter

#### Linux & macOS

```bash
git clone https://github.com/cozyydev/tailchatter.git
cd tailchatter/tailchatter-egui
cargo build --release
./target/release/tailchatter
```

#### Windows

1. Download the `.exe` from [GitHub Releases](https://github.com/cozyydev/tailchatter/releases)
2. Run the `.exe`

## Two Ways to Use

### Option 1: Connect To Existing Server

Use this when connecting to a server running on another node (e.g., your Tailscale network).

1. Launch TailChatter
2. Click **Connect To Existing Server** tab
3. Enter your **Handle** (2-24 chars: letters, numbers, `_` or `-`)
4. Enter the **Server IP** (Tailscale IP or MagicDNS name)
5. Enter the **Port** (default: 42069)
6. Click **Join Chat**

### Option 2: Start Your Own Server

Start a server directly in the app - no separate process needed:

1. Launch TailChatter
2. Click **Create A Server** tab
3. Enter your **Handle**
4. Enter **Server IP** (Tailscale IP or MagicDNS name)
5. Enter **Server Port** (default: 42069)
6. Click **Start Server & Join Chat**

The server runs in the background. If you log out, the server keeps running so others can keep chatting - click **Rejoin Chat** to reconnect.

## Recommended Network Model

TailChatter is designed for private use over Tailscale.

**Setup:**

- Start the server on one machine in your Tailscale network
- Connect clients from other machines using their Tailscale IPs
- No public internet exposure by default

### Getting Your Tailscale IP

On the server machine:

```bash
tailscale ip -4
```

Use that IP when connecting clients, or use the MagicDNS name from your Tailscale admin console.

## Usage

### Group Chat

- Type your message in the input box at the bottom
- Press **Enter** or click **Send**
- Messages appear in the main chat area for all connected users

### Direct Messages

- Click any username in the **Users** sidebar to open a DM conversation
- A new conversation appears in the **Conversations** list on the left
- DMs are private between you and the recipient
- Unread DM count shows next to the conversation name
- Click a conversation to switch between Group Chat and DMs

### Online Users

The left sidebar shows all connected users. Your nickname is highlighted in cyan.

### Logout

Click the **Logout** button at the bottom in the sidebar. If you started a local server, it keeps running - click **Rejoin Chat** to reconnect.

## Features

- Desktop GUI client (egui)
- Built-in TCP server (start directly in app)
- Connect to existing servers or create your own
- Group chat with real-time messages
- Private direct messages between users
- Conversation list with unread counts
- Online user list sidebar
- Rejoin chat when server keeps running
- Dracula-themed UI
- Linux, macOS, and Windows support

## License

MIT
