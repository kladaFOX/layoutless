#!/usr/bin/env python3
"""Download an evaluation corpus for `layoutless eval` into eval-corpus/.

- wiki-ru.txt, wiki-en.txt: intros of random Wikipedia articles
- commands.txt: names of the commands in /usr/bin (what gets typed in terminals)

The corpus is not committed: Wikipedia text is CC-BY-SA, and it's easy to refetch.
"""
import json
import os
import re
import sys
import urllib.parse
import urllib.request

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "eval-corpus")
UA = "layoutless-eval/0.1 (https://github.com/kladaFOX/layoutless)"


def wiki(lang: str, batches: int) -> str:
    texts = []
    for _ in range(batches):
        q = urllib.parse.urlencode({
            "action": "query", "format": "json", "generator": "random",
            "grnnamespace": 0, "grnlimit": 20, "prop": "extracts",
            "exintro": 1, "explaintext": 1, "exlimit": "max",
        })
        req = urllib.request.Request(f"https://{lang}.wikipedia.org/w/api.php?{q}", headers={"User-Agent": UA})
        with urllib.request.urlopen(req, timeout=30) as r:
            pages = json.load(r).get("query", {}).get("pages", {})
        texts += [p.get("extract", "") for p in pages.values()]
    return "\n\n".join(t for t in texts if t)


def commands() -> str:
    names = [n for n in os.listdir("/usr/bin") if re.fullmatch(r"[a-z]+", n)]
    return "\n".join(sorted(names))


def main() -> None:
    batches = int(sys.argv[1]) if len(sys.argv) > 1 else 25
    os.makedirs(OUT, exist_ok=True)
    for lang in ("ru", "en"):
        with open(os.path.join(OUT, f"wiki-{lang}.txt"), "w", encoding="utf-8") as f:
            f.write(wiki(lang, batches))
    with open(os.path.join(OUT, "commands.txt"), "w", encoding="utf-8") as f:
        f.write(commands())


if __name__ == "__main__":
    main()
