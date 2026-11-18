#!/usr/bin/env python3
"""Export the editable SVG masters. Requires Python 3 and rsvg-convert."""

from concurrent.futures import ThreadPoolExecutor
from copy import deepcopy
from pathlib import Path
import json
import shutil
import struct
import subprocess
import xml.etree.ElementTree as ET
import zipfile


ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "source"
NS = "http://www.w3.org/2000/svg"
ET.register_namespace("", NS)
ICON_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 180, 192, 256, 512, 1024, 2048]
CONCEPTS = [
    dict(id="01-monogram", name="Выход", description="Буква Р и отделившийся элемент релиза.",
         primary="#173E35", secondary="#173E35", accent="#95C928", text="#173E35",
         dark_primary="#C6FB50", dark_secondary="#C6FB50", dark_accent="#FFFFFF",
         tile="#C6FB50", tile_primary="#173E35", tile_secondary="#173E35", tile_accent="#173E35"),
    dict(id="02-layers", name="Версии", description="Три слоя, каждый следующий — на шаг выше.",
         primary="#4658F5", secondary="#93A1FF", accent="#C6FB50", text="#202546",
         dark_primary="#A1ADFF", dark_secondary="#687EFF", dark_accent="#C6FB50",
         tile="#4658F5", tile_primary="#FFFFFF", tile_secondary="#C6FB50", tile_accent="#C6FB50"),
    dict(id="03-forward", name="Вперёд", description="Два шеврона: движение к следующему выпуску.",
         primary="#ED5638", secondary="#FFB49C", accent="#ED5638", text="#45261D",
         dark_primary="#FF8B70", dark_secondary="#FFD5C8", dark_accent="#FF8B70",
         tile="#FF7958", tile_primary="#45261D", tile_secondary="#FFF4EC", tile_accent="#45261D"),
]
VARIANTS = ["color", "dark", "black", "white"]
JOBS = []
FILES = []


def element(tag, **attrs):
    return ET.Element(f"{{{NS}}}{tag}", {key.replace("_", "-"): str(value) for key, value in attrs.items()})


def svg(width, height, label):
    node = element("svg", viewBox=f"0 0 {width:g} {height:g}", width=f"{width:g}", height=f"{height:g}",
                   role="img", aria_label=label)
    ET.SubElement(node, f"{{{NS}}}title").text = label
    return node


def colors(concept, variant):
    if variant in ("black", "white"):
        color = "#000000" if variant == "black" else "#FFFFFF"
        return {key: color for key in ("primary", "secondary", "accent", "text")}
    if variant == "dark":
        return {**{key: concept[f"dark_{key}"] for key in ("primary", "secondary", "accent")}, "text": "#FFFFFF"}
    return {key: concept[key] for key in ("primary", "secondary", "accent", "text")}


def mark(parent, concept, palette, x=0, y=0, size=128):
    group = element("g", transform=f"translate({x:g} {y:g}) scale({size / 128:g})")
    for child in ET.parse(SOURCE / f"{concept['id']}.svg").getroot():
        node = deepcopy(child)
        node.set("fill", palette[node.attrib.pop("data-color")])
        group.append(node)
    parent.append(group)


WORDMARK = ET.parse(SOURCE / "wordmark.svg").getroot()
WORD_WIDTH = float(WORDMARK.attrib["viewBox"].split()[2])
HORIZONTAL_WIDTH = 152 + WORD_WIDTH * .58 + 20


def wordmark(parent, color, x, y, height):
    group = element("g", transform=f"translate({x:g} {y:g}) scale({height / 100:g})", color=color)
    for child in WORDMARK:
        group.append(deepcopy(child))
    parent.append(group)


def write_svg(path, root):
    path.parent.mkdir(parents=True, exist_ok=True)
    ET.indent(root, space="  ")
    ET.ElementTree(root).write(path, encoding="utf-8", xml_declaration=True)
    FILES.append(path)


def raster(source, target, width):
    target.parent.mkdir(parents=True, exist_ok=True)
    JOBS.append((source, target, width))
    FILES.append(target)


def export_concept(concept):
    base = ROOT / concept["id"]
    for variant in VARIANTS:
        palette = colors(concept, variant)
        for layout in ("mark", "horizontal", "stacked", "wordmark"):
            if layout == "mark":
                root = svg(128, 128, "Релизы — знак")
                mark(root, concept, palette)
                widths = [512, 1024]
            elif layout == "horizontal":
                root = svg(HORIZONTAL_WIDTH, 128, "Релизы")
                mark(root, concept, palette)
                wordmark(root, palette["text"], 152, 35, 58)
                widths = [400, 800, 1600]
            elif layout == "stacked":
                root = svg(320, 320, "Релизы")
                mark(root, concept, palette, 76, 18, 168)
                wordmark(root, palette["text"], 24, 220, 272 / WORD_WIDTH * 100)
                widths = [512, 1024]
            else:
                root = svg(WORD_WIDTH, 100, "Релизы")
                wordmark(root, palette["text"], 0, 0, 100)
                widths = [800]
            path = base / "svg" / f"{layout}-{variant}.svg"
            write_svg(path, root)
            for width in widths:
                raster(path, base / "png" / f"{layout}-{variant}-{width}.png", width)

    tile_palette = {key: concept[f"tile_{key}"] for key in ("primary", "secondary", "accent")}
    for filename, rounded, mark_size in [("favicon.svg", True, 104), ("app-icon.svg", False, 104), ("maskable.svg", False, 88)]:
        root = svg(128, 128, "Релизы — иконка")
        root.append(element("rect", width=128, height=128, rx=28 if rounded else 0, fill=concept["tile"]))
        margin = (128 - mark_size) / 2
        mark(root, concept, tile_palette, margin, margin, mark_size)
        path = base / "svg" / filename
        write_svg(path, root)
        if filename == "favicon.svg":
            for size in ICON_SIZES:
                raster(path, base / "png" / f"icon-{size}.png", size)
        elif filename == "app-icon.svg":
            raster(path, base / "png" / "apple-touch-icon-180.png", 180)
            for size in (192, 512, 1024, 2048):
                raster(path, base / "png" / f"app-icon-{size}.png", size)
        else:
            for size in (192, 512):
                raster(path, base / "png" / f"maskable-{size}.png", size)


def board():
    root = svg(1800, 1150, "Релизы — три варианта логотипа")
    root.append(element("rect", width=1800, height=1150, fill="#F2F3F0"))
    wordmark(root, "#173E35", 64, 50, 66)

    def label(text, x, y, size=20, color="#69736D", weight="400"):
        node = element("text", x=x, y=y, font_family="Arial, sans-serif", font_size=size, fill=color, font_weight=weight)
        node.text = text
        root.append(node)

    label("ТРИ НАПРАВЛЕНИЯ / SVG + PNG + ICO", 64, 157, 16)
    label("Яркий характер. Простые формы. Любой размер.", 1736, 91, 22)
    root[-1].set("text-anchor", "end")
    for index, concept in enumerate(CONCEPTS):
        x = 64 + index * 568
        root.append(element("rect", x=x, y=204, width=536, height=870, rx=24, fill="#FFFFFF"))
        label(f"0{index + 1} / {concept['name']}", x + 32, 255, 22, "#29392F", "700")
        mark(root, concept, colors(concept, "color"), x + 154, 280, 228)
        wordmark(root, concept["text"], x + 68, 540, 400 / WORD_WIDTH * 100)
        root.append(element("rect", x=x + 24, y=650, width=488, height=242, rx=16, fill="#14231E"))
        mark(root, concept, colors(concept, "dark"), x + 52, 704, 136)
        wordmark(root, "#FFFFFF", x + 209, 750, 270 / WORD_WIDTH * 100)
        # The thumbnail row shows the same icon and a single-color lockup.
        root.append(element("rect", x=x + 32, y=943, width=72, height=72, rx=16, fill=concept["tile"]))
        tile_palette = {key: concept[f"tile_{key}"] for key in ("primary", "secondary", "accent")}
        mark(root, concept, tile_palette, x + 39, 950, 58)
        mark(root, concept, colors(concept, "black"), x + 136, 951, 64)
        wordmark(root, "#000000", x + 220, 971, 270 / WORD_WIDTH * 100)
        label("Иконка", x + 32, 1045, 14)
        label("Одноцветная версия", x + 136, 1045, 14)
    label("Цветной · для тёмного фона · чёрный · белый / иконки 16–2048 px", 64, 1120, 18)
    write_svg(ROOT / "preview.svg", root)
    raster(ROOT / "preview.svg", ROOT / "preview.png", 1800)


def render(job):
    source, target, width = job
    subprocess.run(["rsvg-convert", "--width", str(width), "--keep-aspect-ratio", "--output", str(target), str(source)], check=True)


def ico(concept):
    base = ROOT / concept["id"]
    images = [(size, (base / "png" / f"icon-{size}.png").read_bytes()) for size in (16, 24, 32, 48, 64, 128, 256)]
    offset = 6 + len(images) * 16
    result = bytearray(struct.pack("<HHH", 0, 1, len(images)))
    for size, data in images:
        result.extend(struct.pack("<BBBBHHII", size if size < 256 else 0, size if size < 256 else 0, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    for _, data in images:
        result.extend(data)
    path = base / "favicon.ico"
    path.write_bytes(result)
    FILES.append(path)


def archive(path, files, prefix):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as bundle:
        for file in sorted(files):
            bundle.write(file, Path(prefix) / file.relative_to(ROOT))


def main():
    if shutil.which("rsvg-convert") is None:
        raise SystemExit("Нужен rsvg-convert (librsvg). SVG можно использовать без этого инструмента.")
    for concept in CONCEPTS:
        export_concept(concept)
    board()
    with ThreadPoolExecutor(max_workers=4) as executor:
        list(executor.map(render, JOBS))
    for concept in CONCEPTS:
        ico(concept)
    manifest = dict(name="Релизы", icon_sizes=ICON_SIZES, variants=VARIANTS, concepts=CONCEPTS,
                    files=[str(file.relative_to(ROOT)) for file in sorted(FILES)])
    (ROOT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
    FILES.append(ROOT / "manifest.json")
    for concept in CONCEPTS:
        concept_files = [file for file in FILES if file.is_relative_to(ROOT / concept["id"])]
        concept_files += [ROOT / "README.md"]
        archive(ROOT / f"{concept['id']}.zip", concept_files, "relizy")
    complete = [*FILES, ROOT / "README.md", ROOT / "index.html", *SOURCE.glob("*"), Path(__file__).resolve()]
    archive(ROOT / "relizy-brand-kit.zip", complete, "relizy-brand-kit")
    print(f"Готово: {len(FILES)} файлов, {len(JOBS)} PNG, три варианта и общий ZIP.")


if __name__ == "__main__":
    main()
