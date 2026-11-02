# Troubleshooting

The common problems and their fixes. If yours isn't here, use **Send feedback** (menu bar → Help, or Settings → About): it opens a GitHub issue with your versions filled in. Attaching the log helps (Settings → About → **Show log file**); it never contains what you said or typed.

## Nothing happens when I hold the key

- **Mac:** Splashdown needs **Accessibility** to notice the key. Open Settings → Permissions; if it's off, turn on Splashdown in System Settings → Privacy & Security → Accessibility. After an update, macOS sometimes needs the switch turned off and on again.
- Another app may use the same key. Pick a different dictation key in Settings → General; the picker warns about common clashes.
- **Right Option types accented letters** in some layouts. Choose another key, such as Right Command, Fn or a mouse side button.

## The microphone hears nothing

The pill says **"Microphone hears nothing — muted, or the wrong one?"** after 2.5 seconds of silence.

- Unmute the microphone, or check its input level in your system's sound settings.
- Choose the microphone in Settings → General. "Automatic" uses the system default, but avoids Bluetooth headsets so AirPods don't switch into call mode.
- **Mac:** check Settings → Permissions → Microphone. **Windows:** Settings → Privacy & security → Microphone, with "Let desktop apps access your microphone" on.

If the microphone is unplugged mid-recording, Splashdown stops and types what it heard so far.

## The text didn't appear

- **"Focus changed — copied to clipboard":** you switched windows before the text was ready. Paste it with ⌘V or Ctrl+V.
- **"Couldn't type it — copied to clipboard":** the app wouldn't accept pasted text. Paste it yourself, or switch that app's preset to insert by **Type** (Settings → Presets).
- **Password fields:** Splashdown never dictates into them, by design.
- **Windows:** apps running as administrator don't accept typing from other apps. Splashdown copies the text instead.
- **Chrome and other browsers** sometimes drop a paste in a busy page. Splashdown checks and retries once; if it still happens on a site, tell us which one.

## "Didn't catch that"

Splashdown heard sound but no words: too short, too quiet, or only noise. Hold the key a moment before you start speaking. If you speak quietly, turn on **Whisper mode** (Settings → General, or say “whisper mode on”).

## The AI changed what I meant

- Press **⌃⌥Z** (Ctrl+Alt+Z on Windows) right after it's typed to swap in exactly what you said.
- Lower the cleanup level for that app's preset: **Light** fixes only punctuation and obvious slips, **None** types your words as spoken.
- Smaller on-device models follow instructions less reliably than cloud models. For instructions and commands, turn on Settings → Providers → "Send instructions to the cloud", if you use a cloud key.

## Spoken commands don't work

Commands start with a tap of **Right Shift** before you say anything. They need an AI model, and editing commands ("delete that last sentence") need an app that lets Splashdown read and select its text. If an app doesn't, Splashdown says so instead of guessing.

## A provider error

The pill names the provider and the fix: a rejected key (check it in Settings → Providers), no credit left, rate limits, or the provider being down. If an on-device model is installed, Splashdown retries the dictation on your computer and the result says **"done on this computer"**.

## Windows says "Windows protected your PC"

Choose **More info → Run anyway**. Signed installers show "Point Dynamics, LLC" as the publisher; SmartScreen can still warn about a new release until enough people have downloaded it.

## Starting over

To reset Splashdown completely, quit it and delete its folders, listed in [Privacy](PRIVACY.md#deleting-everything). Settings → About → **Export settings** first if you want to keep your presets and snippets.
