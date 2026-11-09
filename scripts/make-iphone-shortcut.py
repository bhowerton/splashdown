#!/usr/bin/env python3
"""Builds the Splashdown Note shortcut for iPhone and signs it.

The shortcut turns what you say into text (Apple's dictation) and saves it
in iCloud Drive's Shortcuts folder as "Splashdown <date time>.txt".
Splashdown on the computer adds each one to the capture note and deletes it
(app/src-tauri/src/notes/phone.rs). Keep the name and its date format in
step with that file.

Save File without asking saves only into the Shortcuts folder, and takes
its destination path as the file's name, adding .txt. A path ending in a
folder ("Splashdown/") is saved as "Splashdown.txt", not into a folder.

Run on a Mac signed in to iCloud (`shortcuts sign` needs it):

    python3 scripts/make-iphone-shortcut.py

Writes site/iphone/Splashdown Note.shortcut.
"""

import os
import plistlib
import subprocess
import sys
import tempfile
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "site", "iphone", "Splashdown Note.shortcut")

# Shortcuts' placeholder for a variable inside text.
OBJ = "￼"


def output(action_uuid, name):
    """A variable: another action's output."""
    return {"Type": "ActionOutput", "OutputUUID": action_uuid, "OutputName": name}


def attachment(value):
    return {"Value": value, "WFSerializationType": "WFTextTokenAttachment"}


def text(*parts):
    """Text made of strings and variables."""
    string, attachments = "", {}
    for part in parts:
        if isinstance(part, str):
            string += part
        else:
            attachments[f"{{{len(string)}, 1}}"] = part
            string += OBJ
    return {
        "Value": {"string": string, "attachmentsByRange": attachments},
        "WFSerializationType": "WFTextTokenString",
    }


def action(identifier, **parameters):
    return {"WFWorkflowActionIdentifier": identifier, "WFWorkflowActionParameters": parameters}


def build():
    said, when = (str(uuid.uuid4()).upper() for _ in range(2))
    actions = [
        action(
            "is.workflow.actions.dictatetext",
            UUID=said,
            WFDictateTextStopListening="After Pause",
        ),
        action(
            "is.workflow.actions.format.date",
            UUID=when,
            WFDateFormatStyle="Custom",
            WFDateFormat="yyyy-MM-dd HH-mm-ss",
            WFDate=text({"Type": "CurrentDate"}),
        ),
        action(
            "is.workflow.actions.documentpicker.save",
            WFInput=attachment(output(said, "Dictated Text")),
            WFAskWhereToSave=False,
            # Shortcuts adds .txt itself.
            WFFileDestinationPath=text("Splashdown ", output(when, "Formatted Date")),
            WFSaveFileOverwrite=False,
        ),
        action(
            "is.workflow.actions.notification",
            WFNotificationActionTitle="Saved for Splashdown",
            WFNotificationActionBody="It's added to your notes when Splashdown is running on your computer.",
            WFNotificationActionSound=False,
        ),
    ]
    return {
        "WFWorkflowActions": actions,
        "WFWorkflowClientVersion": "2607.0.2",
        "WFWorkflowMinimumClientVersion": 900,
        "WFWorkflowMinimumClientVersionString": "900",
        "WFWorkflowHasOutputFallback": False,
        "WFWorkflowHasShortcutInputVariables": False,
        # A microphone glyph on blue.
        "WFWorkflowIcon": {
            "WFWorkflowIconStartColor": 463140863,
            "WFWorkflowIconGlyphNumber": 59511,
        },
        "WFWorkflowImportQuestions": [],
        "WFWorkflowInputContentItemClasses": [],
        "WFWorkflowOutputContentItemClasses": [],
        "WFWorkflowTypes": ["Watch"],
    }


def main():
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        unsigned = os.path.join(tmp, "Splashdown Note.shortcut")
        with open(unsigned, "wb") as f:
            plistlib.dump(build(), f, fmt=plistlib.FMT_BINARY)
        subprocess.run(
            ["shortcuts", "sign", "--mode", "anyone", "--input", unsigned, "--output", OUT],
            check=True,
        )
    print(f"wrote {os.path.relpath(OUT)}")


if __name__ == "__main__":
    sys.exit(main())
