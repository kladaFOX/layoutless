#!/usr/bin/env python3
"""Rebuild daemon/data/{ru,en}.txt.gz from hermitdave/FrequencyWords.

Downloads the 2018 full lists (OpenSubtitles), keeps words made of letters
only (en may contain one inner apostrophe), folds ё into е, and keeps words
seen at least MIN_COUNT times. Output: "word count" lines, most frequent first.
"""
import collections
import gzip
import os
import re
import urllib.request

MIN_COUNT = 10
URL = "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/{0}/{0}_full.txt"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "daemon", "data")
WORD = {"ru": re.compile(r"[а-яё]+"), "en": re.compile(r"[a-z]+('[a-z]+)?")}

for lang in ("ru", "en"):
    counts = collections.Counter()
    with urllib.request.urlopen(URL.format(lang), timeout=120) as r:
        for line in r.read().decode("utf-8").splitlines():
            parts = line.split()
            if len(parts) != 2 or not WORD[lang].fullmatch(parts[0]):
                continue
            counts[parts[0].replace("ё", "е")] += int(parts[1])
    rows = sorted(((w, c) for w, c in counts.items() if c >= MIN_COUNT), key=lambda x: -x[1])
    with gzip.open(os.path.join(OUT, f"{lang}.txt.gz"), "wt", encoding="utf-8", compresslevel=9) as f:
        f.writelines(f"{w} {c}\n" for w, c in rows)
    print(lang, len(rows))
