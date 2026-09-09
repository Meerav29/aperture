# Application icons

Generated icon assets are checked into this directory, including Windows,
macOS, and PNG variants. These are not placeholders. The current Tauri bundle
configuration references icons/icon.png; platform release packaging still needs
the production signing/notarization and installer checks in the specification.

To regenerate intentionally from an approved source image, run:

```sh
npm run tauri icon path/to/1024.png
```

Review generated asset changes and the bundle icon configuration before committing.
Generating icons alone does not verify a Windows or macOS distribution.
