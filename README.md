# cs-flythrough

An ambient screensaver that loads Counter-Strike 1.6 GoldSrc BSP map files and renders a smooth, continuous first-person camera flythrough — no UI, no HUD, just pure nostalgic exploration.

The experience mirrors the Windows 95 maze screensaver: continuous forward movement through iconic maps like de_dust2, cs_italy, and de_aztec. Powered by the actual map and texture files from your own CS 1.6 or CS: Condition Zero installation — no Valve assets bundled.

---

## Requirements

- Windows 10/11
- Counter-Strike 1.6 **or** Counter-Strike: Condition Zero installed (Steam or standalone)
- DirectX 12 or Vulkan-capable GPU

---

## Installation

1. Download `cs-flythrough.exe` from the [Releases](../../releases) page
2. Place it in a folder of your choice
3. Run it once to generate a default `cs-flythrough.toml` config file
4. Edit `cs-flythrough.toml` and set `cs_install_path` to your CS install directory
5. Run again — the flythrough starts immediately

To exit the screensaver: move the mouse or press any key.

---

## Configuration

`cs-flythrough.toml` lives next to the binary:

```toml
cs_install_path = "C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike"
map_selection = "single"   # "single" | "list" | "all"
map = "de_dust2"
camera_speed = 133.0       # units/sec (CS 1.6 walk speed)
bob_amplitude = 2.0        # camera bob height in units
bob_frequency = 2.0        # bob cycles per second
```

| Setting | Description |
|---|---|
| `cs_install_path` | Root directory of your CS 1.6 or CZ install |
| `map_selection` | `single` — one map; `list` — named maps; `all` — every map found |
| `map` | Map name (without `.bsp`) when `map_selection = "single"` |
| `camera_speed` | Movement speed in GoldSrc units/sec. 133 = CS walk speed |
| `bob_amplitude` | Vertical bob in units. 0 to disable |
| `bob_frequency` | Bob cycles per second |

---

## Map Compatibility

On first load, each map is tested and the result is recorded in `map-compatibility.toml` next to the binary. Maps that fail to parse are automatically excluded from rotation with the exact error logged.

To retry a failed map: remove its entry from `map-compatibility.toml` and run again.

---

## Building from Source

Requires Rust stable 1.77+.

```bash
git clone https://github.com/ktulue/cs-flythrough
cd cs-flythrough
cargo build --release
```

The binary will be at `target/release/cs-flythrough.exe`.

**Running the integration test** (requires a CS install):

```bash
CS_INSTALL_PATH="C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike" cargo test --test bsp_integration -- --nocapture
```

---

## Roadmap

- [x] de_dust2 flythrough (skateboard)
- [ ] Windows `.scr` registration (appears in screensaver control panel)
- [ ] Map selection settings dialog
- [ ] Multi-map rotation
- [ ] Sky dome textures
- [ ] Ambient audio
- [ ] Low-power idle mode

---

## Legal

This project does not bundle any Valve assets. Map files (`.bsp`) and texture files (`.wad`) are read directly from your own Counter-Strike installation and remain the property of Valve Corporation.

---

## License

MIT — see [LICENSE](LICENSE)

---

## Support

☕ [Buy me a coffee on Ko-fi](http://ko-fi.com/ktulue)

Created by Ktulue | The Water Father 🌊
