#!/usr/bin/env python3
"""Generate a compact icon-mapping.json from the upstream material-icon-theme.json.

Usage:
    python3 scripts/generate-icon-mapping.py <upstream-json> <output-json>

Example:
    python3 scripts/generate-icon-mapping.py /tmp/material-icon-theme-upstream.json assets/icons/material/icon-mapping.json
"""

import json
import re
import sys


def strip_path_prefix(path: str) -> str:
    """Remove './icons/' prefix, leaving just the filename."""
    return re.sub(r"^\./icons/", "", path)


def normalize_dir_name(name: str) -> str:
    """Normalize a directory name: lowercase, strip leading ./_ and trailing _."""
    s = name.lower()
    while s and s[0] in "._":
        s = s[1:]
    while s and s[-1] == "_":
        s = s[:-1]
    return s


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(1)

    upstream_path = sys.argv[1]
    output_path = sys.argv[2]

    with open(upstream_path) as f:
        data = json.load(f)

    theme = data["themes"][0]

    # 1. file_icons: icon name -> SVG filename
    file_icons = {}
    for name, val in theme.get("file_icons", {}).items():
        if isinstance(val, dict) and "path" in val:
            file_icons[name] = strip_path_prefix(val["path"])

    # 2. file_suffixes: extension -> icon name (already lowercase in upstream)
    file_suffixes = {}
    for ext, icon_name in theme.get("file_suffixes", {}).items():
        ext_lower = ext.lstrip(".").lower()
        if icon_name in file_icons:
            file_suffixes[ext_lower] = icon_name

    # 3. file_stems: deduplicate to lowercase
    file_stems_raw = theme.get("file_stems", {})
    file_stems = {}
    for stem, icon_name in file_stems_raw.items():
        lower = stem.lower()
        if lower not in file_stems and icon_name in file_icons:
            file_stems[lower] = icon_name

    # 4. directory_icons: default collapsed/expanded
    dir_icons_raw = theme.get("directory_icons", {})
    directory_icons = {
        "collapsed": strip_path_prefix(dir_icons_raw.get("collapsed", "")),
        "expanded": strip_path_prefix(dir_icons_raw.get("expanded", "")),
    }

    # 5. named_directories: deduplicate by normalized name
    named_dirs_raw = theme.get("named_directory_icons", {})
    named_directories = {}
    for name, val in named_dirs_raw.items():
        if not isinstance(val, dict):
            continue
        normalized = normalize_dir_name(name)
        if not normalized or normalized in named_directories:
            continue
        collapsed = strip_path_prefix(val.get("collapsed", ""))
        expanded = strip_path_prefix(val.get("expanded", ""))
        if collapsed and expanded:
            named_directories[normalized] = [collapsed, expanded]

    result = {
        "file_icons": file_icons,
        "file_suffixes": file_suffixes,
        "file_stems": file_stems,
        "directory_icons": directory_icons,
        "named_directories": named_directories,
    }

    with open(output_path, "w") as f:
        json.dump(result, f, separators=(",", ":"))

    # Stats
    print(f"file_icons:        {len(file_icons)}")
    print(f"file_suffixes:     {len(file_suffixes)}")
    print(f"file_stems:        {len(file_stems)}")
    print(f"named_directories: {len(named_directories)}")
    size_kb = len(json.dumps(result, separators=(",", ":"))) / 1024
    print(f"Output size:       {size_kb:.1f} KB")
    print(f"Written to:        {output_path}")


if __name__ == "__main__":
    main()
