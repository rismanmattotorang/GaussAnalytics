# README images

- **`tui-console.png`** — a faithful render of the admin TUI. It is produced from
  the **actual** widget tree: `cargo run -p gauss-tui --example screenshot` draws
  the console via ratatui's `TestBackend` and serializes the resulting cell buffer
  to `tui-console.svg`, which is rasterized to PNG. Regenerate with:

  ```bash
  cargo run -p gauss-tui --example screenshot > docs/img/tui-console.svg
  python3 -c "import cairosvg; cairosvg.svg2png(url='docs/img/tui-console.svg', write_to='docs/img/tui-console.png', scale=2.0)"
  ```

- **`web-ui.png`** — a high-fidelity **UI preview** of the React web app, authored
  in SVG (`web-ui.svg`) using the app's real design tokens (the palette, gradient
  brand, nav, and nivo-style chart from `frontend/src/styles.css`) and rasterized
  to PNG. It mirrors the shipped design rather than being a live browser capture.
