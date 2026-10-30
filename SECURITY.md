# Security

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue: on the repository's **Security** tab, choose **Report a vulnerability**. Include what you found, how to reproduce it, and which version and OS you used. You'll get a reply within a week.

## What's in scope

Splashdown sees a lot: it hears the microphone while the hotkey is held, reads the focused text field through the accessibility APIs, uses the clipboard, and stores API keys. Problems we especially want to hear about:

- API keys leaking (to logs, files, other apps, or a provider other than the one they belong to).
- Dictated text or field contents going anywhere other than the provider the user chose, or being written to logs.
- Prompt injection that makes the AI pass do something other than edit the user's text: for example, text in a selection or a web page that gets obeyed.
- The app inserting text into a different field or app than the one the user dictated into.
- Anything that lets another program use Splashdown's Accessibility or Microphone permission.

## How data is handled

- API keys are stored in the OS keychain (macOS Keychain, Windows Credential Manager). `.env.local` files are for development only.
- Audio and text go only to the speech and AI providers chosen in Settings, directly from your computer. Splashdown runs no servers and collects no telemetry.
- In local-only mode, cloud providers are refused in configuration resolution (`crates/splashdown-config`), not just hidden in the UI.
- History stays in a file on your computer and can be turned off.
