# 🔥 Open Miami // Rogue Purge 🔥

A neon-noir, top-down purge-'em-up written in Rust and running in the browser using WebAssembly!

You are **CL-4UDE**, a friendly coral-colored Claude bot deployed into the compromised
Miami Datacenter. Thirteen floors of racks have gone dark, their resident models drifted
rogue and hostile. Walk every floor, decommission every glitching AI, grab whatever weapon
the last one dropped, and reach the extraction elevator. It's goofy. It's stylish. It's a
very bad night to be a rogue AI. (See [LORE.md](LORE.md) for the full fiction.)

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- **Top-down fast-paced combat** - Purge rogue AI models in brutal, Hotline-Miami-quick fights
- **Play as a Claude bot** - A friendly coral purge bot with a single glowing visor
- **Three rogue archetypes** - Sentinels (red), Drifters (violet) and Hunters (magenta)
- **Browser-based** - Runs entirely in your web browser using WebAssembly
- **Written in Rust** - Leveraging Rust's performance and safety
- **Open Source** - MIT licensed, free to use and modify

## Gameplay

- **WASD** - Move your character
- **Mouse** - Aim
- **Left Click** - Shoot
- **E** - Pick up / swap the weapon you're standing on
- **1-4** - Switch weapon
- **R** - Restart after death

### Current Features

- Player (Claude bot) movement with WASD controls
- Camera following the player
- Rogue AI with detection and chase behavior
- Shooting mechanics with limited ammo
- Melee combat system
- Health system for both the Claude bot and the rogues
- Rogues drop their weapon when decommissioned; pick it up to swap (Hotline Miami style)
- 13 hand-designed floors of the Miami Datacenter
- Reboot on death
- Checkered floor pattern for visual reference

## Building and Running

### Prerequisites

- Rust (install from [rustup.rs](https://rustup.rs/))
- For WASM builds: `cargo install cargo-make`

### Running Locally (Native)

The fastest way to test the game:

```bash
cargo run --release
```

### Building for the Web (WASM)

#### Prerequisites

1. Add the WASM target:
```bash
rustup target add wasm32-unknown-unknown
```

2. Install wasm-bindgen-cli (required for generating JavaScript glue code):
```bash
cargo install wasm-bindgen-cli
```

#### Quick Build (Recommended)

Use the provided build script:

```bash
./build-wasm.sh
```

This will:
1. Build the WASM binary
2. Generate the wasm-bindgen JavaScript glue code
3. Prepare all files for deployment

#### Manual Build

If you prefer to build manually:

```bash
# Build the WASM binary
cargo build --release --target wasm32-unknown-unknown

# Generate wasm-bindgen JavaScript glue
wasm-bindgen target/wasm32-unknown-unknown/release/open_miami.wasm \
    --out-dir . \
    --target web \
    --no-typescript
```

#### Running the Game

After building, serve the game with any static file server:

```bash
# Using Python
python3 -m http.server 8000

# Or using Node.js
npx http-server

# Or any other static file server
```

Then open `http://localhost:8000` in your browser.

## Development

The project is structured using a custom Entity-Component-System (ECS) architecture:

```
open-miami/
├── src/
│   ├── main.rs              # Main game loop
│   ├── lib.rs               # Library exports
│   ├── ecs/                 # Custom ECS engine
│   │   ├── entity.rs        # Entity (unique IDs)
│   │   ├── component.rs     # Component trait system
│   │   ├── world.rs         # World/storage management
│   │   ├── query.rs         # Query system for entities
│   │   └── system.rs        # System trait
│   ├── components/          # Game data components
│   │   └── mod.rs           # Position, Health, Weapon, AI, etc.
│   ├── systems/             # Game logic systems
│   │   ├── movement.rs      # Movement logic
│   │   ├── ai.rs            # Enemy AI
│   │   ├── combat.rs        # Combat and damage
│   │   ├── weapon.rs        # Weapon updates
│   │   └── input.rs         # Player input handling
│   ├── game.rs              # Entity spawning helpers
│   ├── render.rs            # Rendering system
│   └── legacy/              # Deprecated OOP code (reference)
├── tests/
│   └── integration_tests.rs # 89 comprehensive tests
├── index.html               # Web interface
├── build-wasm.sh            # WASM build script
├── Cargo.toml               # Rust dependencies
├── ECS_ARCHITECTURE.md      # Detailed ECS documentation
├── TESTING.md               # Testing strategy guide
└── README.md                # This file
```

For detailed information about the ECS architecture and design decisions, see [ECS_ARCHITECTURE.md](ECS_ARCHITECTURE.md).

## Roadmap

Future improvements planned:

- [ ] More weapon types (shotgun, machine gun)
- [ ] Different enemy types
- [ ] Multiple levels/rooms
- [ ] Wall collision
- [ ] Particle effects and blood splatter
- [ ] Sound effects and music
- [ ] Weapon pickup system
- [ ] Score tracking
- [ ] Better graphics and animations
- [ ] Mobile touch controls

## Technology

- **Rust** - Systems programming language
- **wasm-bindgen** - WebAssembly bindings for Rust
- **WebAssembly** - Run Rust in the browser
- **Custom ECS** - Entity-Component-System architecture built from scratch

## Contributing

Contributions are welcome! Feel free to:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Inspiration

This project is inspired by [Hotline Miami](https://en.wikipedia.org/wiki/Hotline_Miami) by Dennaton Games. This is a fan project and is not affiliated with or endorsed by the original creators.

## Credits

Created by [c4ffein](https://github.com/c4ffein)

---

**Purge the rogues. Reach the elevator. EXFILTRATE.** 🎮
