#!/usr/bin/env python3
"""
Interactive Demo Asset Generator
Traces to: [REQ-PUB-001], G-21

Programmatically renders a high-resolution animated terminal GIF demonstrating:
1. Orchestrator bootstrap via Podman Compose
2. Worker enrollment via ephemeral QR token
3. Parallel WASI image benchmark dispatch
4. Real-time multi-core execution progress and monotonic fencing consensus
"""

import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont


WIDTH = 920
HEIGHT = 580
BG_COLOR = (15, 23, 42)          # Slate 900
HEADER_COLOR = (30, 41, 59)      # Slate 800
BORDER_COLOR = (51, 65, 85)      # Slate 700
TEXT_MAIN = (241, 245, 249)      # Slate 100
TEXT_MUTED = (148, 163, 184)     # Slate 400
TEXT_CYAN = (56, 189, 248)       # Sky 400
TEXT_GREEN = (74, 222, 128)      # Emerald 400
TEXT_YELLOW = (250, 204, 21)     # Amber 400
TEXT_PURPLE = (192, 132, 252)    # Purple 400
BAR_FILLED = (56, 189, 248)      # Sky 400
BAR_BG = (30, 41, 59)            # Slate 800


def get_fonts():
    try:
        font_main = ImageFont.truetype("consola.ttf", 15)
        font_bold = ImageFont.truetype("consolab.ttf", 15)
        font_title = ImageFont.truetype("segoeui.ttf", 13)
    except Exception:
        font_main = ImageFont.load_default()
        font_bold = font_main
        font_title = font_main
    return font_main, font_bold, font_title


def draw_window_frame(draw, width, height, title, font_title):
    # Terminal Window Shadow & Background
    draw.rounded_rectangle([0, 0, width - 1, height - 1], radius=8, fill=BG_COLOR, outline=BORDER_COLOR, width=1)
    # Header bar
    draw.rounded_rectangle([0, 0, width - 1, 38], radius=8, fill=HEADER_COLOR)
    draw.rectangle([0, 30, width - 1, 38], fill=HEADER_COLOR)
    draw.line([(0, 38), (width - 1, 38)], fill=BORDER_COLOR, width=1)

    # Window control dots (macOS / Linux style)
    draw.ellipse([14, 13, 26, 25], fill=(239, 68, 68))   # Red
    draw.ellipse([34, 13, 46, 25], fill=(245, 158, 11))  # Yellow
    draw.ellipse([54, 13, 66, 25], fill=(16, 185, 129))  # Green

    # Window title
    draw.text((width // 2 - 160, 11), title, font=font_title, fill=TEXT_MUTED)


def create_frame(step_index: int, sub_progress: float = 0.0) -> Image.Image:
    img = Image.new("RGB", (WIDTH, HEIGHT), BG_COLOR)
    draw = ImageDraw.Draw(img)
    font_main, font_bold, font_title = get_fonts()

    draw_window_frame(draw, WIDTH, HEIGHT, "AndroGrid Cluster Orchestration & Worker Showcase", font_title)

    y = 52
    line_h = 22

    def draw_line(text, color=TEXT_MAIN, bold=False, indent=0):
        nonlocal y
        f = font_bold if bold else font_main
        draw.text((24 + indent, y), text, font=f, fill=color)
        y += line_h

    # Step 1: Bootstrapping orchestrator
    draw_line("$ podman compose up -d", color=TEXT_CYAN, bold=True)
    if step_index >= 1:
        draw_line("[+] Running 2/2", color=TEXT_MUTED)
        draw_line(" ✔ Network androgrid_default           Created", color=TEXT_GREEN)
        draw_line(" ✔ Container androgrid-orchestrator     Started", color=TEXT_GREEN)
        draw_line("[INFO] AndroGrid Host Control Plane v0.1.0 online (Tokio 1.40 / Tonic 0.12)", color=TEXT_MAIN)
        draw_line("[INFO] Listening on 0.0.0.0:50051 (Mutual TLS 1.3 enforced)", color=TEXT_MAIN)
        draw_line("[INFO] Web Dashboard: http://127.0.0.1:8080 | Artifact CAS Engine ready", color=TEXT_MAIN)
        y += 8

    # Step 2: Device enrollment via QR
    if step_index >= 2:
        draw_line("$ androgrid-cli nodes enroll --format qr", color=TEXT_CYAN, bold=True)
        draw_line("[INFO] Generated ephemeral enrollment token: tk_7f9b2c8e14a0 (TTL: 300s)", color=TEXT_YELLOW)
        draw_line("  ┌──────────────────────────────────────────────┐", color=TEXT_PURPLE)
        draw_line("  │  ████████  ██    ██  ████████    [QR CODE]   │", color=TEXT_PURPLE)
        draw_line("  │  ██    ██  ████      ██    ██   Scan to Pair │", color=TEXT_PURPLE)
        draw_line("  │  ████████  ██  ██    ████████   Android Node │", color=TEXT_PURPLE)
        draw_line("  └──────────────────────────────────────────────┘", color=TEXT_PURPLE)
        y += 8

    # Step 3: Device connected & enrolled
    if step_index >= 3:
        draw_line("[SUCCESS] Handshake verified: mTLS 1.3 ECDSA P-256", color=TEXT_GREEN, bold=True)
        draw_line("[INFO] Node registered: worker-pixel7-arm64 (Wi-Fi LAN)", color=TEXT_MAIN)
        draw_line("       Battery: 88% (Charging, 80% ceiling guard) | Temp: 31 C (Hysteresis Safe)", color=TEXT_MUTED, indent=12)
        draw_line("       Hardware: 8 cores (ARM Cortex-X1/A78/A55), 8 GB RAM, Chicory WASI-v1", color=TEXT_MUTED, indent=12)
        y += 8

    # Step 4: Workload benchmark submission
    if step_index >= 4:
        draw_line("$ androgrid-cli benchmark --image input.png --tiles 8 --filter blur", color=TEXT_CYAN, bold=True)
        draw_line("[INFO] Partitioned input.png (3840x2160) into 8 compute tiles", color=TEXT_MAIN)
        draw_line("[INFO] Distributed across cluster workers (1x Pixel 7, 1x Galaxy S22)...", color=TEXT_YELLOW)
        y += 6

        # Progress bars
        tiles = [
            ("Tile #1 (Y:0-270)  ", "worker-pixel7-arm64", 1.0, 182),
            ("Tile #2 (Y:270-540)", "worker-pixel7-arm64", 1.0, 179),
            ("Tile #3 (Y:540-810)", "worker-s22-arm64   ", 1.0, 191),
            ("Tile #4 (Y:810-1080)", "worker-s22-arm64   ", 1.0, 185),
            ("Tile #5 (Y:1080-1350)", "worker-pixel7-arm64", 1.0, 174),
            ("Tile #6 (Y:1350-1620)", "worker-pixel7-arm64", 1.0, 180),
            ("Tile #7 (Y:1620-1890)", "worker-s22-arm64   ", 1.0, 188),
            ("Tile #8 (Y:1890-2160)", "worker-s22-arm64   ", 1.0, 176),
        ]

        # In steps 4 to 11, progress increments
        p_factor = sub_progress if step_index == 4 else 1.0

        for idx, (tile_name, node_name, max_p, latency_ms) in enumerate(tiles):
            # Calculate individual tile progress
            tile_p = min(1.0, max(0.0, (p_factor * 8.0) - idx))
            pct = int(tile_p * 100)

            # Render progress bar
            bar_len = 20
            filled_len = int(bar_len * tile_p)
            bar_str = "[" + "#" * filled_len + "-" * (bar_len - filled_len) + "]"

            status_color = TEXT_GREEN if pct == 100 else TEXT_CYAN
            time_str = f"{latency_ms}ms" if pct == 100 else "running"

            line_text = f" {tile_name} {bar_str} {pct:3d}% [{node_name}] {time_str}"
            draw_line(line_text, color=status_color, indent=8)

        if step_index >= 5:
            y += 4
            draw_line("[SUCCESS] 8/8 tiles assembled into output.png in 1.42s | Throughput: 5.63 tiles/s", color=TEXT_GREEN, bold=True)
            draw_line("[INFO] Monotonic attempt fencing [INV-004] verified: 0 stale commit conflicts", color=TEXT_CYAN)

    return img


def generate_animated_gif(output_path: Path):
    print(f"[INFO] Generating high-resolution demo GIF: {output_path}")
    frames = []
    durations = []

    # Sequence timeline:
    # 1. Start compose (1 frame, 1200 ms)
    frames.append(create_frame(0))
    durations.append(1200)

    # 2. Orchestrator online (1 frame, 1500 ms)
    frames.append(create_frame(1))
    durations.append(1500)

    # 3. QR code generated (1 frame, 1800 ms)
    frames.append(create_frame(2))
    durations.append(1800)

    # 4. Device enrolled (1 frame, 1800 ms)
    frames.append(create_frame(3))
    durations.append(1800)

    # 5. Benchmark dispatch & progress animation (10 frames)
    for i in range(11):
        progress = i / 10.0
        frames.append(create_frame(4, sub_progress=progress))
        durations.append(250)

    # 6. Completed summary with full stats (1 frame, 3500 ms pause)
    frames.append(create_frame(5, sub_progress=1.0))
    durations.append(3500)

    # Convert to adaptive 256-color palette to minimize file size
    quantized_frames = []
    for f in frames:
        quantized = f.convert("P", palette=Image.Palette.ADAPTIVE, colors=128)
        quantized_frames.append(quantized)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    quantized_frames[0].save(
        output_path,
        save_all=True,
        append_images=quantized_frames[1:],
        duration=durations,
        loop=0,
        optimize=True,
    )

    size_bytes = output_path.stat().st_size
    print(f"[SUCCESS] Demo GIF generated at {output_path} ({size_bytes} bytes / {size_bytes / 1024:.1f} KB)")


if __name__ == "__main__":
    out = Path(__file__).resolve().parent.parent.parent / "docs" / "assets" / "demo.gif"
    generate_animated_gif(out)
