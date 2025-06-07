#!/bin/bash
# Build script for testing cross-compilation locally
# This helps verify builds before pushing release tags

set -e

echo "🔨 Building DiffScope for all supported targets..."

# Array of targets to build
TARGETS=(
    "x86_64-unknown-linux-gnu"
    "x86_64-unknown-linux-musl"
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
)

# Create output directory
mkdir -p dist

# Build each target
for target in "${TARGETS[@]}"; do
    echo "Building for $target..."
    
    # Check if we can build for this target
    if rustup target list | grep -q "$target (installed)"; then
        cargo build --release --target "$target"
        
        # Copy binary to dist folder
        if [[ "$target" == *"windows"* ]]; then
            cp "target/$target/release/diffscope.exe" "dist/diffscope-$target.exe" || true
        else
            cp "target/$target/release/diffscope" "dist/diffscope-$target" || true
        fi
    else
        echo "⚠️  Target $target not installed. Run: rustup target add $target"
    fi
done

echo "✅ Build complete! Binaries are in the dist/ directory:"
ls -la dist/