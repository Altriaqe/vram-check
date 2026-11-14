"""生成 app.ico：深色底 + 显存芯片 + 红色警戒点。

不需要额外依赖，只用 Pillow。改图标后重新跑：
    python scripts/make_icon.py
"""

from PIL import Image, ImageDraw

SIZES = [256, 128, 64, 48, 32, 24, 16]
SCALE = 4  # 先画大图再缩小，边缘更干净

BG = (11, 18, 32, 255)
BORDER = (30, 41, 59, 255)
CHIP = (15, 118, 110, 255)
CHIP_EDGE = (45, 212, 191, 255)
DOT = (239, 68, 68, 255)


def draw_master(size: int) -> Image.Image:
    s = size * SCALE
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    def r(value: float) -> int:
        return int(round(value * s))

    # 底板
    d.rounded_rectangle(
        (r(0.04), r(0.04), r(0.96), r(0.96)),
        radius=r(0.20),
        fill=BG,
        outline=BORDER,
        width=max(1, r(0.012)),
    )

    # 芯片本体
    left, top, right, bottom = 0.25, 0.25, 0.75, 0.75
    d.rounded_rectangle(
        (r(left), r(top), r(right), r(bottom)),
        radius=r(0.06),
        fill=CHIP,
        outline=CHIP_EDGE,
        width=max(2, r(0.022)),
    )

    # 八根引脚（每边两根）
    pin_long, pin_short = 0.10, 0.035
    for offset in (0.36, 0.64):
        d.rectangle((r(offset - pin_short / 2), r(left - pin_long), r(offset + pin_short / 2), r(left)), fill=CHIP_EDGE)
        d.rectangle((r(offset - pin_short / 2), r(right), r(offset + pin_short / 2), r(right + pin_long)), fill=CHIP_EDGE)
        d.rectangle((r(left - pin_long), r(offset - pin_short / 2), r(left), r(offset + pin_short / 2)), fill=CHIP_EDGE)
        d.rectangle((r(right), r(offset - pin_short / 2), r(right + pin_long), r(offset + pin_short / 2)), fill=CHIP_EDGE)

    # 芯片里的 2x2 小格
    cell = 0.16
    gap = 0.04
    for cx in (0.5 - gap / 2 - cell, 0.5 + gap / 2):
        for cy in (0.5 - gap / 2 - cell, 0.5 + gap / 2):
            d.rounded_rectangle(
                (r(cx), r(cy), r(cx + cell), r(cy + cell)),
                radius=r(0.03),
                fill=(153, 246, 228, 255),
            )

    # 右上角警戒点
    d.ellipse(
        (r(0.68), r(0.10), r(0.82), r(0.24)),
        fill=DOT,
        outline=(254, 226, 226, 255),
        width=max(1, r(0.012)),
    )
    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    masters = [draw_master(size) for size in SIZES]
    masters[0].save(
        "app.ico",
        format="ICO",
        sizes=[(size, size) for size in SIZES],
        append_images=masters[1:],
    )
    masters[0].save("docs/assets/icon-256.png")
    masters[0].resize((64, 64), Image.LANCZOS).save("docs/assets/icon-64.png")
    print("app.ico written")


if __name__ == "__main__":
    main()
