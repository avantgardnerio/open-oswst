#!/usr/bin/python3
"""Make the board ready for JLCPCB, start to finish, with no clicking:

1. board.py generates open-oswst.kicad_pcb (the source of truth)
2. freerouting routes it: exported as Specctra .dsn, routed headless, the
   routes (.ses) imported back
3. every zone filled, and the routed board saved to out/route/
4. DRC on the routed board: any violation (warnings too, unconnected items
   included) stops here, so nothing is plotted from a dirty board
5. Gerbers (F/B copper, silkscreen, mask, and Edge.Cuts) and drill files
   (PTH and NPTH apart) into out/, checked, and zipped there for upload
6. the routed board opened in KiCad for a last look

System Python on purpose: KiCad's `pcbnew` module is installed for it, not
for the repo's venv. Commit first: the silkscreen carries the commit's hash,
so a dirty tree gets a warning (the board wouldn't say what it was made from).

Usage: pcb/make.py [--no-open]
"""

import subprocess
import sys
import zipfile
from pathlib import Path

import pcbnew

PCB = Path(__file__).resolve().parent
NAME = "open-oswst"
OUT = PCB / "out"
ROUTE = OUT / "route"  # .dsn, .ses, the routed board, the DRC report
# What JLCPCB gets: these files in out/, and the zip of them
FAB_FILES = ("*.gbr", "*.gbrjob", "*.drl", "*.zip")

FREEROUTING = (Path.home() / ".local/share/kicad/9.0/3rdparty/plugins"
               / "app_freerouting_kicad-plugin/jar/freerouting-2.4.1.jar")
# Passes before freerouting stops: the board routes in a handful
ROUTING_PASSES = 100

GERBER_LAYERS = ["F.Cu", "B.Cu", "F.Silkscreen", "B.Silkscreen",
                 "F.Mask", "B.Mask", "Edge.Cuts"]
# Mounting holes, the only non-plated holes
NPTH_HOLES = 4


def main():
    commit = git_commit()

    print("1. Generating the board")
    run([sys.executable, "board.py"], cwd=PCB)

    print("2. Routing (freerouting)")
    ROUTE.mkdir(parents=True, exist_ok=True)
    board = pcbnew.LoadBoard(str(PCB / f"{NAME}.kicad_pcb"))
    dsn, ses = ROUTE / f"{NAME}.dsn", ROUTE / f"{NAME}.ses"
    ses.unlink(missing_ok=True)
    if not pcbnew.ExportSpecctraDSN(board, str(dsn)):
        fail("Specctra export failed")
    run(["java", "-jar", str(FREEROUTING),
         "-de", str(dsn), "-do", str(ses), "-mp", str(ROUTING_PASSES),
         # Headless, and nothing about the board leaves this machine
         "--gui.enabled=false",
         "--usage_and_diagnostic_data.disable_analytics=true",
         "--profile.allow_telemetry=false"])
    if not ses.exists():
        fail("freerouting wrote no session (.ses)")
    if not pcbnew.ImportSpecctraSES(board, str(ses)):
        fail("Session import failed")

    print("3. Filling zones")
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    routed = ROUTE / f"{NAME}.kicad_pcb"
    board.Save(str(routed))
    print(f"   {len(board.GetTracks())} tracks and vias")

    print("4. DRC")
    report = ROUTE / "drc.rpt"
    drc = subprocess.run(
        ["kicad-cli", "pcb", "drc", "--severity-all", "--exit-code-violations",
         "--output", str(report), str(routed)])
    for line in report.read_text().splitlines():
        if line.startswith("** Found"):
            print("   " + line.strip("* "))
    if drc.returncode != 0:
        fail(f"DRC not clean: {report}")

    print("5. Gerbers and drill files")
    # Last run's files go first, so nothing stale gets zipped or uploaded
    for pattern in FAB_FILES:
        for old in OUT.glob(pattern):
            old.unlink()
    run(["kicad-cli", "pcb", "export", "gerbers", "--no-protel-ext",
         "--layers", ",".join(GERBER_LAYERS), "--output", f"{OUT}/", str(routed)])
    run(["kicad-cli", "pcb", "export", "drill", "--format", "excellon",
         "--excellon-separate-th", "--output", f"{OUT}/", str(routed)])
    fab = check_fab()
    archive = OUT / f"{NAME}-{commit}.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zipped:
        for path in fab:
            zipped.write(path, path.name)
    print(f"   {archive} ({archive.stat().st_size // 1024} KB): upload this")

    if "--no-open" not in sys.argv:
        print("6. Opening the routed board in KiCad")
        subprocess.Popen(["pcbnew", str(routed)], start_new_session=True,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def git_commit():
    """HEAD's short hash, which board.py prints on the silkscreen. Warns if
    the tree has changes the hash doesn't cover"""
    commit = subprocess.check_output(["git", "rev-parse", "--short", "HEAD"],
                                     cwd=PCB, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain", "--", "."],
                                    cwd=PCB, text=True).strip()
    if dirty:
        print(f"WARNING: uncommitted changes in pcb/: the silkscreen says {commit},"
              " but the board isn't exactly that commit")
    return commit


def check_fab():
    """Every layer has its Gerber, and both drill files are there; the hole
    counts are printed for comparing with the board. Returns the files"""
    fab = sorted(path for pattern in FAB_FILES[:-1] for path in OUT.glob(pattern))
    names = {path.name for path in fab}
    for layer in GERBER_LAYERS:
        gerber = f"{NAME}-{layer.replace('.', '_')}.gbr"
        if gerber not in names:
            fail(f"No Gerber for {layer} ({gerber})")
    for drill in (f"{NAME}-PTH.drl", f"{NAME}-NPTH.drl"):
        if drill not in names:
            fail(f"No drill file {drill}")
    plated = hits(OUT / f"{NAME}-PTH.drl")
    unplated = hits(OUT / f"{NAME}-NPTH.drl")
    if unplated != NPTH_HOLES:
        fail(f"{unplated} unplated holes, expected {NPTH_HOLES} (the mounting holes)")
    print(f"   {len(names)} files, {plated} plated holes (pads and vias),"
          f" {unplated} unplated")
    return fab


def hits(drill_file):
    """Holes in an Excellon file: one coordinate line each"""
    return sum(1 for line in drill_file.read_text().splitlines()
               if line.startswith(("X", "Y")))


def run(command, cwd=None):
    if subprocess.run(command, cwd=cwd).returncode != 0:
        fail(f"Failed: {' '.join(str(part) for part in command)}")


def fail(message):
    sys.exit(f"STOPPED: {message}")


if __name__ == "__main__":
    main()
