# 🔥 Open Miami 🔥

An open-source Hotline Miami clone written in Rust and running in the browser using WebAssembly!

![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)

## Features

- **Top-down fast-paced combat** - Inspired by Hotline Miami's brutal and quick gameplay
- **Browser-based** - Runs entirely in your web browser using WebAssembly
- **Written in Rust** - Leveraging Rust's performance and safety
- **Open Source** - MIT licensed, free to use and modify

## Gameplay

- **WASD** - Move your character
- **Mouse** - Aim
- **Left Click** - Shoot
- **Right Click** - Melee attack
- **R** - Restart after death

### Current Features

- Player movement with WASD controls
- Camera following the player
- Enemy AI with detection and chase behavior
- Shooting mechanics with limited ammo
- Melee combat system
- Health system for both player and enemies
- Level restart on death
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

1. Add the WASM target:
```bash
rustup target add wasm32-unknown-unknown
```

2. Build the WASM bundle:
```bash
cargo build --release --target wasm32-unknown-unknown
```

3. Serve the game (you'll need a local web server):
```bash
# Using Python
python3 -m http.server 8000

# Or using any other static file server
```

4. Open `http://localhost:8000` in your browser

### Quick WASM Build Script

For convenience, you can use the provided build script:

```bash
./build-wasm.sh
```

This will build the WASM version and provide instructions for serving it.

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
- **macroquad** - Simple and easy to use game library
- **WebAssembly** - Run Rust in the browser

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

**Have fun and enjoy the mayhem!** 🎮
