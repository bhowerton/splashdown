# Splashdown for teams

Two things make Splashdown work for a group of people, and neither needs a server or an account:

- **A shared vocabulary:** one file of names, terms and corrections that everyone's Splashdown reads.
- **Managed settings:** choices an organization makes for its computers, such as "only models on this computer".

## A shared vocabulary

One list of names, terms and corrections, kept as a file in a folder your team already shares (Dropbox, Google Drive, OneDrive, iCloud Drive or Box). When someone adds a name to it, everyone has it at their next dictation.

### Starting one

Settings → Words & language → Vocabulary → **Your team's words** → **Start a team list…**

1. Choose a folder your team shares.
2. Say whether to start the list with your own words or empty.
3. Splashdown makes `Splashdown team words.txt` there, starts using it, and copies an invite link.
4. Paste the link to your team.

### Joining one

Click the invite link. Splashdown says where the list is and how many words it has, and asks before using it.

The link only says where the file is ("Dropbox › Team › Splashdown team words.txt"), so the folder has to be shared with you and synced to your computer first. If the shared folder sits somewhere else in your drive, Splashdown looks a few folders down for the file.

On Windows, or if the link doesn't open the app: copy the link, then choose **Join with an invite link…** in the same place in Settings.

### Changing the list

**Edit the list** in Settings opens the file in a text editor. A name or term per line. A line with a comma is a correction: what gets written by mistake, then what it should be. Lines that start with `# ` are notes.

```text
# Our product and people names.
Kubernetes
Priya Raman
Okta
cooper netties,Kubernetes
pria,Priya
```

Any word list works, not only one Splashdown made: **Choose a file…** takes a text or CSV file, or a dictionary export from another dictation app. From a terminal:

```bash
splashdown-cli settings set general.shared_vocabulary "~/Team Drive/Splashdown words.txt"
```

Settings shows how many words and corrections are in use, or why the file isn't being used (for example, the folder hasn't synced yet).

### How it behaves

- **Your own words come first.** The team's words are added after yours. Where you and the team both correct the same words, yours is used.
- **Nothing is copied into your settings.** The words stay in the file. Stop using the file and they're gone; export your settings and they aren't in it.
- **Changes arrive by themselves.** Splashdown looks at the file every half minute and uses the new one. No restart.
- **It's read, never written.** Words Splashdown learns from your corrections go to your own vocabulary, not the team's file.
- **There are limits.** The first 300 words and the first 300 corrections are used, and an entry longer than 60 characters is left out. The whole vocabulary is sent with every dictation, so a short list of the names people really say works better than a long one.

Anyone who can edit the file changes what everyone's dictation writes, so give edit access the way you would for any shared document.

The invite link holds the name of the sync service and the file's path inside it, nothing else: no words, no account, no key. It's a page on splashdown.app with those two parts after the `#`, which browsers don't send to the site; the page hands them to Splashdown on your computer. A link can't point outside your own sync folders, and nothing is used until you say yes.

## Managed settings

An organization can set some of Splashdown's settings on the computers it manages, with the tools it already uses for other apps. Splashdown shows each one in Settings as "Set by your organization", and it can't be changed there.

| Name | Value | What it does |
|---|---|---|
| `LocalOnly` | true | Speech is recognized on the computer and the AI only runs on the computer or a local server. Nothing said is sent to a cloud provider, even if a key is saved. |
| `KeepHistory` | false | No history of dictations is kept, and no recordings with it. |
| `SharedVocabulary` | a file's path | Everyone uses this vocabulary file (above). |
| `Updates` | `off`, `notify` or `install` | `off`: Splashdown never looks for updates (you deploy them yourself). `notify`: it tells people when there's one. `install`: it installs them by itself when idle. |

Each of these only narrows what the app does. `LocalOnly` set to false, or `KeepHistory` set to true, is ignored: the setting is then each person's own choice.

Managed settings take effect the next time Splashdown starts.

### macOS

Deploy a configuration profile with custom settings for the preference domain `app.splashdown`. Most management tools have a "custom settings" or "preference domain" payload that takes a property list like this one:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>LocalOnly</key>
  <true/>
  <key>KeepHistory</key>
  <false/>
  <key>SharedVocabulary</key>
  <string>/Volumes/Team/Splashdown words.txt</string>
  <key>Updates</key>
  <string>notify</string>
</dict>
</plist>
```

Splashdown reads the managed preferences macOS writes from the profile, for the computer and for the signed-in user. Where both set a value, the computer's is used.

### Windows

Set values under `Software\Policies\Splashdown`, in `HKEY_LOCAL_MACHINE` for the computer or `HKEY_CURRENT_USER` for a person, with Group Policy or your management tool. True and false are a `REG_DWORD` of 1 or 0; the others are `REG_SZ`.

```bat
reg add "HKLM\Software\Policies\Splashdown" /v LocalOnly /t REG_DWORD /d 1 /f
reg add "HKLM\Software\Policies\Splashdown" /v KeepHistory /t REG_DWORD /d 0 /f
reg add "HKLM\Software\Policies\Splashdown" /v Updates /t REG_SZ /d notify /f
```

### What this is, and isn't

Managed settings are the app honouring its organization's choices: the app and `splashdown-cli` both follow them, wherever a setting is read or saved. They aren't a lock against someone who can install other software on the computer. If that matters, restrict what can be installed, as you would for any app.

Splashdown has no accounts, so there is no sign-in to manage, and it sends nothing to its makers, so there is no data-processing agreement to sign with them. With `LocalOnly`, nothing said leaves the computer at all. [Privacy](PRIVACY.md) has the full list of what goes where.

If something here doesn't behave as described with your management tool, please [open an issue](https://github.com/bhowerton/splashdown/issues): this is new.
