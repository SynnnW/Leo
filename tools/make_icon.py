"""Buat assets/amora.ico: kotak gelap membulat dengan notch putih dan titik hijau.

Jalankan: python tools/make_icon.py  (butuh Pillow)
"""
from pathlib import Path

from PIL import Image, ImageDraw

SIZE = 1024  # digambar besar lalu diperkecil supaya halus
BG = (17, 17, 19, 255)
NOTCH = (255, 255, 255, 255)
DOT = (89, 217, 140, 255)


def draw() -> Image.Image:
    img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Latar: kotak membulat seperti ikon aplikasi Windows 11.
    pad = SIZE * 0.06
    d.rounded_rectangle([pad, pad, SIZE - pad, SIZE - pad], radius=SIZE * 0.22, fill=BG)
    # Notch: pil putih menempel di tepi atas latar.
    w, h = SIZE * 0.56, SIZE * 0.2
    x0, y0 = (SIZE - w) / 2, pad + SIZE * 0.2
    d.rounded_rectangle([x0, y0, x0 + w, y0 + h], radius=h / 2, fill=NOTCH)
    # Titik hijau di ujung kanan notch.
    r = h * 0.24
    cx, cy = x0 + w - h * 0.5, y0 + h / 2
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=DOT)
    # Dua "mata" maskot di bawah notch.
    ew, eh = SIZE * 0.07, SIZE * 0.13
    for ex in (SIZE * 0.4, SIZE * 0.6):
        top = SIZE * 0.6
        d.rounded_rectangle([ex - ew / 2, top, ex + ew / 2, top + eh], radius=ew / 2, fill=NOTCH)
    return img


def main() -> None:
    out = Path(__file__).resolve().parent.parent / "assets" / "amora.ico"
    out.parent.mkdir(exist_ok=True)
    big = draw()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    big.resize((256, 256), Image.LANCZOS).save(out, sizes=[(s, s) for s in sizes])
    big.resize((256, 256), Image.LANCZOS).save(out.with_suffix(".png"))
    print(f"ditulis: {out}")


if __name__ == "__main__":
    main()
