"""Fetch the CC0 traditional deck. Source and license retained alongside assets."""
from pathlib import Path
from urllib.request import urlretrieve
root = Path(__file__).resolve().parent.parent / "static/art/cards"
root.mkdir(parents=True, exist_ok=True)
base = "https://raw.githubusercontent.com/letele/playing-cards/main/"
for suit in "SHDC":
    for rank in ["9", "10", "J", "Q", "K", "A"]:
        urlretrieve(base + f"assets/{suit}-{rank}.svg", root / f"{rank.replace('10','T')}{suit}.svg")

urlretrieve(base + "LICENSE", root / "LICENSE.txt")
