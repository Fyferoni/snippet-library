**Download `Snippet-Library.dmg`**, open it, and drag Snippet Library into Applications.

This build isn't notarized by Apple, so macOS blocks it the first time. To open it:

1. Double-click Snippet Library. When macOS says it can't be opened, click **Done**.
2. Open **System Settings > Privacy & Security**, scroll down, and click **Open Anyway** next to Snippet Library.

If macOS instead says the app "is damaged", run this once in Terminal:

```
xattr -dr com.apple.quarantine "/Applications/Snippet Library.app"
```

Your library is saved to `iCloud Drive/Snippet Library/snippets.json`, with daily backups in the `Backups` folder next to it.
