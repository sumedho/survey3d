#!/usr/bin/env python3
"""Generates the tutorial's sample job: Lot 12 Banksia Street, a fictional
residential lot in MGA2020 zone 50 (Perth). Deterministic, so the data and the
tutorial's numbers always agree.

Writes into tutorial/data/:
  control.csv      control marks (name, E, N, Z, code, description)
  detail.csv       the field-coded detail survey, in survey order
  lot12_plan.txt   the lot's plan bearings and distances (for 'bd')
  shed_design.csv  four corners of a proposed shed to set out
"""

import math
import os
import random

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "data")

random.seed(12)

# The lot in a local frame: u along the frontage, v into the lot.
ORIGIN = (391520.000, 6465210.000)  # south-west corner
FRONT_BEARING = 89 + 47 / 60 + 30 / 3600  # SW -> SE
FRONTAGE = 20.115
DEPTH = 40.235


def dms(deg):
    """Degrees to (d, m, s) with seconds rounded to 10"."""
    total = round(deg * 360) * 10  # tenths of a minute -> seconds in 10" steps
    d, rem = divmod(total, 3600)
    m, s = divmod(rem, 60)
    return int(d) % 360, int(m), int(s)


def dms_text(deg):
    d, m, s = dms(deg)
    return f"{d}°{m:02d}'{s:02d}\""


def dms_input(deg):
    d, m, s = dms(deg)
    return f"{d}.{m:02d}{s:02d}"


def from_dms(d, m, s):
    return d + m / 60 + s / 3600


def to_world(u, v):
    b = math.radians(FRONT_BEARING)
    # u along the frontage bearing, v at the bearing 90 degrees anticlockwise.
    ue, un = math.sin(b), math.cos(b)
    ve, vn = math.sin(b - math.pi / 2), math.cos(b - math.pi / 2)
    return ORIGIN[0] + u * ue + v * ve, ORIGIN[1] + u * un + v * vn


def bearing(a, b):
    return math.degrees(math.atan2(b[0] - a[0], b[1] - a[1])) % 360


def dist(a, b):
    return math.hypot(b[0] - a[0], b[1] - a[1])


def ground(u, v):
    """Natural surface: rising gently to the rear, with a 0.75 m bank."""
    z = 24.35 + 0.025 * v + 0.004 * u + 0.03 * math.sin(u / 3.1) * math.cos(v / 4.3)
    if v >= 32.5:
        z += 0.75
    elif v > 31.0:
        z += 0.75 * (v - 31.0) / 1.5
    return z


def kerb_z(u):
    return 24.12 + 0.006 * u


def noise(mm):
    return random.uniform(-mm, mm) / 1000


def main():
    os.makedirs(DATA, exist_ok=True)

    # The true corners. The east side is a little off square, as old lots are.
    sw = to_world(0, 0)
    se = to_world(FRONTAGE, 0)
    nw = to_world(0, DEPTH)
    ne = to_world(FRONTAGE + 0.012, DEPTH + 0.004)
    corners = [("SW", sw, 0, 0), ("NW", nw, 0, DEPTH), ("NE", ne, FRONTAGE, DEPTH), ("SE", se, FRONTAGE, 0)]

    # Control: two survey marks in the road, one on the far verge.
    control = [
        ("1001", to_world(-12.0, -9.5), "SSM 50417"),
        ("1002", to_world(34.0, -9.8), "SSM 50418"),
        ("1003", to_world(-6.0, -14.5), "PM 2286"),
    ]
    with open(os.path.join(DATA, "control.csv"), "w") as f:
        f.write("Name,Easting,Northing,Height,Code,Description\n")
        for name, (e, n), desc in control:
            u, v = ((-12.0, -9.5), (34.0, -9.8), (-6.0, -14.5))[int(name) - 1001]
            f.write(f"{name},{e:.3f},{n:.3f},{kerb_z(u) - 0.03:.3f},CP,{desc}\n")

    rows = []

    def shot(u, v, z, code, desc=""):
        e, n = to_world(u, v)
        rows.append((e + noise(4), n + noise(4), z + noise(3), code, desc))

    # Kerb and edge of pavement along the road.
    for i, u in enumerate([-10, -2, 6, 14, 22, 30]):
        shot(u, -4.2, kerb_z(u), "KB ST" if i == 0 else "KB", "Kerb" if i == 0 else "")
    for i, u in enumerate([-10, -2, 6, 14, 22, 30]):
        shot(u, -4.6, kerb_z(u) - 0.15, "EP ST" if i == 0 else "EP")
    shot(17.5, -2.0, ground(17.5, -2.0), "MHS", "Sewer manhole")

    # Boundary pegs found (true corners, plus measurement noise).
    for label, (e, n), u, v in corners:
        rows.append((e + noise(4), n + noise(4), ground(u, v) + noise(3), "PEG", f"Peg found {label}"))

    # Fences, just inside the boundary: west side, rear, east side.
    fence = [(0.08, 0.4), (0.10, 20.0), (0.10, 40.12), (10.0, 40.14), (20.03, 40.15), (20.05, 20.0), (20.04, 0.4)]
    for i, (u, v) in enumerate(fence):
        code = "FNC ST" if i == 0 else ("FNC END" if i == len(fence) - 1 else "FNC")
        shot(u, v, ground(u, v), code, "Colorbond fence" if i == 0 else "")

    # The house: three corners, the fourth squared off by RECT.
    shot(1.6, 7.0, ground(1.6, 7.0) + 0.3, "BLD ST", "House")
    shot(11.6, 7.0, ground(11.6, 7.0) + 0.3, "BLD")
    shot(11.6, 21.0, ground(11.6, 21.0) + 0.3, "BLD RECT")

    # The rear bank: bottom and top.
    for i, u in enumerate([0.6, 7.0, 13.0, 19.5]):
        shot(u, 31.0, ground(u, 31.0), "BB ST" if i == 0 else "BB")
    for i, u in enumerate([0.6, 7.0, 13.0, 19.5]):
        shot(u, 32.5, ground(u, 32.5), "TB ST" if i == 0 else "TB")

    # Trees.
    for u, v, desc in [(16.5, 4.0, "Jacaranda spread 6"), (4.0, 36.5, "Lemon tree"), (15.5, 37.0, "Peppermint spread 5")]:
        shot(u, v, ground(u, v), "TR", desc)

    # Natural surface spot heights on a rough grid, clear of the house.
    for v in [1.5, 5.0, 10.0, 15.0, 20.0, 25.0, 28.5, 34.5, 38.5]:
        for u in [1.0, 6.0, 10.5, 15.0, 19.0]:
            uu, vv = u + random.uniform(-0.6, 0.6), v + random.uniform(-0.6, 0.6)
            if 1.3 < uu < 11.9 and 6.7 < vv < 21.3:
                continue
            shot(uu, vv, ground(uu, vv), "NS")
    for u in [-8, 0, 8, 16, 24]:  # verge
        shot(u, -2.0, ground(u, -2.0) - 0.05, "NS")

    with open(os.path.join(DATA, "detail.csv"), "w") as f:
        f.write("Name,Easting,Northing,Height,Code,Description\n")
        for i, (e, n, z, code, desc) in enumerate(rows, start=1):
            f.write(f"{i},{e:.3f},{n:.3f},{z:.3f},{code},{desc}\n")

    # The plan: bearings to 10", distances to 5 mm, clockwise from SW.
    legs = []
    ring = [sw, nw, ne, se, sw]
    names = ["SW", "NW", "NE", "SE", "SW"]
    for a, b, na, nb in zip(ring, ring[1:], names, names[1:]):
        brg = bearing(a, b)
        d = round(dist(a, b) / 0.005) * 0.005
        legs.append((na, nb, brg, d))
    with open(os.path.join(DATA, "lot12_plan.txt"), "w") as f:
        f.write("Lot 12 on Deposited Plan 41275 (fictional) - Banksia Street\n")
        f.write("Bearings MGA2020 zone 50, distances in metres. Clockwise from the south-west corner.\n\n")
        for na, nb, brg, d in legs:
            f.write(f"{na} -> {nb}   {dms_text(brg):>14}   {d:8.3f}      bd input: {dms_input(brg)} {d:.3f}\n")

    # A 4 m x 3 m garden shed in the rear yard, square to the boundary.
    with open(os.path.join(DATA, "shed_design.csv"), "w") as f:
        f.write("Name,Easting,Northing,Height,Code,Description\n")
        for i, (u, v) in enumerate([(13.0, 34.5), (17.0, 34.5), (17.0, 37.5), (13.0, 37.5)], start=1):
            e, n = to_world(u, v)
            f.write(f"S{i},{e:.3f},{n:.3f},{ground(u, v) + 0.1:.3f},SHED,Shed corner {i}\n")

    print(f"{len(rows)} detail points")
    print(open(os.path.join(DATA, "lot12_plan.txt")).read())


if __name__ == "__main__":
    main()
