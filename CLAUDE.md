# Rules for working on this repo

- Work silently. Final message: 5 lines max, no essay.
- NEVER permanently delete user files. Never overwrite a file. Moves only after the user confirms a preview.
- The library (`core/`, crate `organizer-core`) contains NO GUI code. The GUI (`src/`) only draws and calls the library.
- Run the tests (`cargo test --workspace`) before every push. Real Windows build + tests run in GitHub Actions (`.github/workflows/build.yml`); keep it green.
- Every safety rule needs a test in `core/tests/`.
- Keep dependencies few and well-known; keep the exe small.
- Stage 1 = sort loose files. Stage 2 (duplicates, big/old files) and stage 3 (renaming, tray auto-sort) come later as new modules in `core/`.
