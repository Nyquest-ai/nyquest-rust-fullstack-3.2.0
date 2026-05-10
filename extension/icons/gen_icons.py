#!/usr/bin/env python3
"""Generate Nyquest extension icons at 16, 48, 128px."""
import subprocess, sys, os

ICON_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)))
SVG = os.path.join(ICON_DIR, "icon.svg")

for size in [16, 48, 128]:
    out = os.path.join(ICON_DIR, f"icon-{size}.png")
    try:
        subprocess.run([
            "rsvg-convert", "-w", str(size), "-h", str(size),
            SVG, "-o", out
        ], check=True)
        print(f"Created {out}")
    except FileNotFoundError:
        # Fallback: try convert (imagemagick)
        try:
            subprocess.run([
                "convert", "-background", "none",
                "-resize", f"{size}x{size}", SVG, out
            ], check=True)
            print(f"Created {out} (imagemagick)")
        except FileNotFoundError:
            # Last resort: create a simple PNG with PIL
            try:
                from PIL import Image, ImageDraw, ImageFont
                img = Image.new('RGBA', (size, size), (10, 11, 14, 255))
                draw = ImageDraw.Draw(img)
                # Draw rounded rect background
                draw.rounded_rectangle([(0,0),(size-1,size-1)], radius=size//6, fill=(10,11,14,255))
                # Draw NQ text
                fs = max(6, size // 3)
                try:
                    font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf", fs)
                except:
                    font = ImageFont.load_default()
                bbox = draw.textbbox((0,0), "NQ", font=font)
                tw = bbox[2] - bbox[0]
                th = bbox[3] - bbox[1]
                draw.text(((size-tw)//2, (size-th)//2 - size//8), "NQ", fill=(79,209,197,255), font=font)
                img.save(out)
                print(f"Created {out} (PIL)")
            except ImportError:
                print(f"SKIP {out} - no rsvg-convert, convert, or PIL available")

print("Done.")
