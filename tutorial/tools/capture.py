#!/usr/bin/env python3
"""Works through the tutorial in the running survey3d window and takes its
screenshots (tutorial/images) and output files (tutorial/output).

Start survey3d with Settings -> General -> Agent link on, size the window to
about 1440 x 900, then:  python3 tutorial/tools/capture.py
It starts a new project (discarding the open one) and must not be disturbed.
"""

import json
import os
import shutil
import subprocess

from drive import App

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.abspath(os.path.join(HERE, "..", "data"))
OUT = os.path.abspath(os.path.join(HERE, "..", "output"))
IMAGES = os.path.abspath(os.path.join(HERE, "..", "images"))

PLAN_LEGS = ["359.4730 40.235", "89.4650 20.125", "179.4830 40.240", "269.4730 20.115"]


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(OUT)
    app = App()
    log = {}

    # 1. The interface.
    app.call("new_project", discard_changes=True)
    app.run("closeall", "plan", "ze")
    app.shot("01-interface")

    # 2. Project settings.
    app.call("set_project", name="Lot 12 Banksia Street", conventions="Australian",
             coordinate_system={"name": "MGA2020 zone 50"},
             description="Detail and boundary survey, Lot 12 Banksia Street")
    app.run("conventions")
    app.shot("02-conventions")
    app.run("closeall")

    # 3. Feature codes.
    app.call("set_feature_codes", preset="example", codes=[{
        "code": "PEG", "description": "Boundary peg", "layer": "BOUNDARY MARKS",
        "color": "#ea580c", "linework": False, "style": {"symbol": "boundary_mark"}}])
    app.run("codes")
    app.shot("03-feature-codes")
    app.run("closeall")

    # 4. Importing the survey.
    log["import_control"] = app.call("import_file", path=f"{DATA}/control.csv", layout="PENZCD")
    log["import_detail"] = app.call("import_file", path=f"{DATA}/detail.csv", layout="PENZCD")
    app.run("ze")
    app.shot("04-imported")
    app.run("points")
    app.shot("04-points-table")
    app.run("closeall")

    # 5. Linework from codes.
    app.call("select", kind="points")
    log["linework"] = app.run("linework")
    app.call("select", mode="clear")
    app.shot("05-linework")
    app.shot("05-layers", crop="right")

    # 6. The lot boundary by bearing and distance.
    app.call("set_layer", layer="BOUNDARY", color="#dc2626", current=True, line_weight=2.5)
    log["bd"] = app.run("bd", "14", *PLAN_LEGS, "c", "Lot 12", "")
    app.shot("06-bd-command-line", crop="bottom")
    app.shot("06-lot")
    app.run("parcels")
    app.shot("06-parcels")
    app.run("closeall")

    # 7. Checking the computed corners against the pegs found.
    log["inverse"] = app.run("inverse", "15", "1004", "")
    log["inverse_ne"] = app.run("inverse", "16", "1005", "")
    app.shot("07-inverse", crop="bottom")

    # 8. The ground surface and contours, clipped to the lot.
    lot = json.loads(app.call("query", kind="polylines", filter={"layer": "BOUNDARY"}))["rows"][0]["id"]
    log["surface"] = app.call(
        "create_surface", name="Existing ground", boundary=lot,
        point_codes=["NS", "PEG", "TB", "BB", "KB", "EP", "TR", "FNC"],
        breakline_codes=["TB", "BB", "KB", "EP"],
        style={"contours": {"show": True, "interval": 0.25, "major_every": 4, "labels": True},
               "elevation_colors": True})
    app.run("ze")
    app.shot("08-surface")
    app.call("set_layer", layer="BOUNDARY", visible=False)
    app.run("iso")
    app.shot("08-surface-3d", wait=1.0)
    app.run("plan", "ze")
    app.call("set_layer", layer="BOUNDARY", visible=True)

    # 9. Volumes: levelling the lot to RL 25.30.
    app.call("select", kind="polylines", filter={"ids": [lot]})
    app.run("volumes")
    app.shot("09-volumes-window")
    app.run("closeall")
    log["volumes"] = app.call("compute_volumes", base="Existing ground", top_level=25.3, boundary=lot)
    app.shot("09-volumes-report")
    app.run("closeall")
    app.call("select", mode="clear")

    # 10. Setting out the shed.
    app.call("set_layer", layer="DESIGN", color="#0891b2", current=True)
    log["import_shed"] = app.call("import_file", path=f"{DATA}/shed_design.csv", layout="PENZCD", layer="DESIGN")
    log["setout"] = app.call("setout", station="1001", backsight="1002",
                             points=["S1", "S2", "S3", "S4"], hi=1.55, ht=1.8, report=True)
    app.shot("10-setout-report")
    app.run("closeall")
    app.run("stakeout")
    app.shot("10-stakeout-window")
    app.run("closeall")
    app.call("select", kind="points", filter={"layer": "DESIGN"})
    log["stake_export"] = app.call("export_file", format="csv", selection=True, path=f"{OUT}/shed_setout.csv")
    app.call("select", mode="clear")

    # 11. A plan sheet.
    sheet = json.loads(app.call("set_record", kind="sheet", record={
        "name": "Detail survey", "scale": 250, "paper": "a3", "landscape": True,
        "centre": [391538.0, 6465222.0],
        "title": {"title": "Lot 12 Banksia Street", "subtitle": "Detail and boundary survey",
                  "surveyor": "A. Surveyor", "reference": "Job 2026-041"}}))
    app.run("sheets")
    app.shot("11-sheets-window", wait=1.0)
    app.run("closeall")
    app.call("render", sheet_id=sheet["id"], width=1800, save_image="11-sheet")
    subprocess.run(["magick", os.path.join(IMAGES, "11-sheet.png"), "-rotate", "-90",
                    os.path.join(IMAGES, "11-sheet-rotated.png")], check=True)
    log["pdf"] = app.call("export_file", format="pdf", sheet_id=sheet["id"], path=f"{OUT}/lot12_detail_survey.pdf")

    # 12. Saving, undo and the audit trail.
    log["save"] = app.call("save_project", path=f"{OUT}/lot12.s3d")
    app.run("audit")
    app.shot("12-audit", wait=1.0)
    app.run("closeall")
    app.shot("12-finished")

    with open(os.path.join(OUT, "capture-log.json"), "w") as f:
        json.dump(log, f, indent=2, ensure_ascii=False)
    print(json.dumps(log, indent=2, ensure_ascii=False)[:6000])


if __name__ == "__main__":
    main()
