#!/usr/bin/env python3
"""
Skrypt budowania EXRuster.app — macOS .app bundle
Autor: Projekt EXRuster

Użycie:
  python build_mac.py                # Pełny build release + .app bundle
  python build_mac.py --skip-build   # Tylko pakowanie (binarka musi już istnieć)
  python build_mac.py --open         # Otwórz folder z .app po zbudowaniu
"""

import subprocess
import sys
import os
import shutil
import plistlib
import argparse
import time
import re
import tempfile
from pathlib import Path
from typing import Optional


# ─── Konfiguracja ────────────────────────────────────────────────────
APP_NAME = "EXRuster"
BUNDLE_ID = "com.conceptfab.exruster"
BUNDLE_SIGNATURE = "EXRu"
MIN_MACOS = "11.0"
CARGO_BIN_NAME = "EXruster"
ICON_SOURCE = "resources/img/icon.png"
# Typy plików obsługiwane przez aplikację (rejestracja w Finderze)
SUPPORTED_EXTENSIONS = ["exr", "hdr"]
# ─────────────────────────────────────────────────────────────────────


class MacAppBuilder:
    def __init__(self, project_dir: str = "."):
        self.project_dir = Path(project_dir).resolve()
        self.cargo_toml = self.project_dir / "Cargo.toml"
        self.dist_dir = self.project_dir / "dist"
        self.app_path = self.dist_dir / f"{APP_NAME}.app"

    # ── Helpers ──────────────────────────────────────────────────────

    @staticmethod
    def _print_header(msg: str):
        print(f"\n{'='*60}")
        print(f"  {msg}")
        print(f"{'='*60}")

    @staticmethod
    def _print_step(n: int, msg: str):
        print(f"\n[{n}] {msg}")
        print("-" * 40)

    def _read_version(self) -> str:
        """Odczytuje wersję z Cargo.toml."""
        try:
            content = self.cargo_toml.read_text(encoding="utf-8")
        except Exception:
            return "0.0.0"
        # Próba z tomllib (Python 3.11+)
        try:
            import tomllib
            data = tomllib.loads(content)
            ver = data.get("package", {}).get("version")
            if isinstance(ver, str):
                return ver.strip()
        except Exception:
            pass
        # Fallback regex
        m = re.search(r'^\s*version\s*=\s*"([^"]+)"', content, re.MULTILINE)
        return m.group(1) if m else "0.0.0"

    def _run(self, cmd: list[str], desc: str, live: bool = False) -> bool:
        """Uruchamia komendę. Zwraca True przy sukcesie."""
        print(f"🔄 {desc}...")
        print(f"   $ {' '.join(cmd)}")
        t0 = time.time()
        try:
            if live:
                r = subprocess.run(cmd, cwd=self.project_dir, check=True)
            else:
                r = subprocess.run(
                    cmd, cwd=self.project_dir,
                    capture_output=True, text=True, check=True,
                )
                if r.stdout.strip():
                    for line in r.stdout.strip().splitlines()[:10]:
                        print(f"   {line}")
            elapsed = time.time() - t0
            print(f"✅ {desc} — {elapsed:.1f}s")
            return True
        except subprocess.CalledProcessError as e:
            elapsed = time.time() - t0
            print(f"❌ {desc} — błąd po {elapsed:.1f}s (kod {e.returncode})")
            if hasattr(e, "stderr") and e.stderr:
                for line in e.stderr.strip().splitlines()[-15:]:
                    print(f"   {line}")
            return False

    # ── Krok 1: Kompilacja Rust ──────────────────────────────────────

    def cargo_build(self) -> bool:
        self._print_step(1, "Kompilacja cargo build --release")
        return self._run(
            ["cargo", "build", "--release", "--bin", CARGO_BIN_NAME],
            f"Kompilacja '{CARGO_BIN_NAME}' (release)",
            live=True,
        )

    # ── Krok 2: Generowanie .icns ────────────────────────────────────

    def generate_icns(self, out_path: Path) -> bool:
        """Generuje plik .icns z PNG źródłowego za pomocą sips + iconutil."""
        self._print_step(2, "Generowanie ikony .icns")

        src_png = self.project_dir / ICON_SOURCE
        if not src_png.exists():
            print(f"⚠️  Brak ikony źródłowej: {src_png}")
            print("   Pomijam generowanie .icns — aplikacja będzie miała domyślną ikonę.")
            return True  # Nie jest to błąd krytyczny

        iconset_dir = Path(tempfile.mkdtemp()) / "AppIcon.iconset"
        iconset_dir.mkdir(parents=True)

        # Rozmiary ikon wymagane przez macOS
        sizes = [16, 32, 64, 128, 256, 512]
        ok = True
        for sz in sizes:
            # Wersja 1x
            target = iconset_dir / f"icon_{sz}x{sz}.png"
            r = subprocess.run(
                ["sips", "-z", str(sz), str(sz), str(src_png), "--out", str(target)],
                capture_output=True, text=True,
            )
            if r.returncode != 0:
                print(f"   ❌ sips błąd dla {sz}x{sz}: {r.stderr.strip()}")
                ok = False

            # Wersja 2x (Retina) — poza 512 (bo 1024 nie jest wymagane formalnie,
            # ale warto mieć dla 512@2x)
            if sz <= 512:
                sz2 = sz * 2
                target2 = iconset_dir / f"icon_{sz}x{sz}@2x.png"
                r2 = subprocess.run(
                    ["sips", "-z", str(sz2), str(sz2), str(src_png), "--out", str(target2)],
                    capture_output=True, text=True,
                )
                if r2.returncode != 0:
                    print(f"   ❌ sips błąd dla {sz}x{sz}@2x: {r2.stderr.strip()}")
                    ok = False

        if not ok:
            print("⚠️  Niektóre rozmiary ikon nie zostały wygenerowane")

        # iconutil -> .icns
        r = subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(out_path)],
            capture_output=True, text=True,
        )
        # Cleanup
        shutil.rmtree(iconset_dir.parent, ignore_errors=True)

        if r.returncode != 0:
            print(f"❌ iconutil błąd: {r.stderr.strip()}")
            return False

        print(f"✅ Wygenerowano ikonę: {out_path} ({out_path.stat().st_size / 1024:.0f} KB)")
        return True

    # ── Krok 3: Budowa .app bundle ───────────────────────────────────

    def assemble_app(self) -> bool:
        self._print_step(3, f"Budowa {APP_NAME}.app bundle")

        version = self._read_version()

        # Ścieżka do skompilowanej binarki
        release_bin = self.project_dir / "target" / "release" / CARGO_BIN_NAME
        if not release_bin.exists():
            print(f"❌ Nie znaleziono binarki: {release_bin}")
            print("   💡 Uruchom bez --skip-build lub wykonaj: cargo build --release")
            return False

        # Czyszczenie starego bundle
        if self.app_path.exists():
            print(f"🗑️  Usuwam stary bundle: {self.app_path}")
            shutil.rmtree(self.app_path)

        # Tworzenie struktury katalogów
        contents_dir = self.app_path / "Contents"
        macos_dir = contents_dir / "MacOS"
        resources_dir = contents_dir / "Resources"

        macos_dir.mkdir(parents=True)
        resources_dir.mkdir(parents=True)

        # 3a: Kopiowanie binarki
        print(f"📦 Kopiowanie binarki do MacOS/")
        dest_bin = macos_dir / APP_NAME
        shutil.copy2(release_bin, dest_bin)
        dest_bin.chmod(0o755)
        size_mb = dest_bin.stat().st_size / (1024 * 1024)
        print(f"   {dest_bin.name}: {size_mb:.1f} MB")

        # 3b: Generowanie ikony
        icns_path = resources_dir / "AppIcon.icns"
        self.generate_icns(icns_path)

        # 3c: Generowanie Info.plist
        print(f"📝 Generowanie Info.plist (wersja {version})")
        plist = {
            "CFBundleName": APP_NAME,
            "CFBundleDisplayName": APP_NAME,
            "CFBundleIdentifier": BUNDLE_ID,
            "CFBundleVersion": version,
            "CFBundleShortVersionString": version,
            "CFBundleExecutable": APP_NAME,
            "CFBundleIconFile": "AppIcon",
            "CFBundlePackageType": "APPL",
            "CFBundleSignature": BUNDLE_SIGNATURE,
            "LSMinimumSystemVersion": MIN_MACOS,
            "NSHighResolutionCapable": True,
            "LSUIElement": False,
            "NSSupportsAutomaticGraphicsSwitching": True,
            # Rejestracja typów plików — pozwala na "Otwórz za pomocą…" w Finderze
            "CFBundleDocumentTypes": [
                {
                    "CFBundleTypeName": ext.upper() + " Image",
                    "CFBundleTypeRole": "Viewer",
                    "LSHandlerRank": "Alternate",
                    "LSItemContentTypes": [f"public.{ext}" if ext == "hdr" else "com.ilm.openexr-image"],
                    "CFBundleTypeExtensions": [ext],
                }
                for ext in SUPPORTED_EXTENSIONS
            ],
        }
        plist_path = contents_dir / "Info.plist"
        with open(plist_path, "wb") as f:
            plistlib.dump(plist, f)

        print(f"✅ Info.plist zapisany")
        return True

    # ── Krok 4: Weryfikacja ──────────────────────────────────────────

    def verify(self) -> bool:
        self._print_step(4, "Weryfikacja bundle")
        ok = True

        expected = [
            self.app_path / "Contents" / "Info.plist",
            self.app_path / "Contents" / "MacOS" / APP_NAME,
        ]
        for p in expected:
            if p.exists():
                print(f"   ✓ {p.relative_to(self.app_path)}")
            else:
                print(f"   ✗ BRAK: {p.relative_to(self.app_path)}")
                ok = False

        icns = self.app_path / "Contents" / "Resources" / "AppIcon.icns"
        if icns.exists():
            print(f"   ✓ {icns.relative_to(self.app_path)} ({icns.stat().st_size / 1024:.0f} KB)")
        else:
            print(f"   ⚠ {icns.relative_to(self.app_path)} — brak (domyślna ikona)")

        # Sprawdź czy binarka jest poprawna Mach-O
        r = subprocess.run(
            ["file", str(self.app_path / "Contents" / "MacOS" / APP_NAME)],
            capture_output=True, text=True,
        )
        if "Mach-O" in r.stdout:
            arch = "arm64" if "arm64" in r.stdout else "x86_64" if "x86_64" in r.stdout else "?"
            print(f"   ✓ Mach-O executable ({arch})")
        else:
            print(f"   ✗ Nie jest plikiem Mach-O!")
            ok = False

        if ok:
            total_size = sum(
                f.stat().st_size for f in self.app_path.rglob("*") if f.is_file()
            ) / (1024 * 1024)
            print(f"\n🎉 {self.app_path.name} gotowy! ({total_size:.1f} MB)")
            print(f"   📍 {self.app_path}")
        return ok

    # ── Pełny pipeline ───────────────────────────────────────────────

    def build(self, skip_build: bool = False, open_folder: bool = False) -> bool:
        self._print_header(f"🍎 BUDOWANIE {APP_NAME}.app")
        print(f"📁 Projekt: {self.project_dir}")
        print(f"📦 Wyjście: {self.dist_dir}/")
        print(f"🔖 Wersja:  {self._read_version()}")

        t0 = time.time()

        # Krok 1: Kompilacja
        if not skip_build:
            if not self.cargo_build():
                return False
        else:
            print("\n⏭️  Pomijam kompilację (--skip-build)")

        # Krok 2+3: Budowa bundle (ikona generowana wewnątrz)
        if not self.assemble_app():
            return False

        # Krok 4: Weryfikacja
        ok = self.verify()

        elapsed = time.time() - t0
        print(f"\n⏱️  Cały proces: {elapsed:.1f}s")

        if ok and open_folder:
            subprocess.run(["open", str(self.dist_dir)])

        return ok


def main():
    parser = argparse.ArgumentParser(
        description=f"Budowanie {APP_NAME}.app — macOS application bundle",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=f"""
Przykłady:
  python build_mac.py                  # Pełny build + bundle
  python build_mac.py --skip-build     # Tylko pakowanie (binarka musi istnieć)
  python build_mac.py --open           # Build + otwórz folder w Finderze
  python build_mac.py --project-dir .  # Inny katalog projektu
        """,
    )
    parser.add_argument(
        "--skip-build", action="store_true",
        help="Pomiń kompilację cargo (binarka musi już istnieć w target/release/)",
    )
    parser.add_argument(
        "--open", action="store_true",
        help="Otwórz folder dist/ w Finderze po zbudowaniu",
    )
    parser.add_argument(
        "--project-dir", type=str, default=".",
        help="Ścieżka do katalogu projektu (domyślnie: bieżący)",
    )

    args = parser.parse_args()

    builder = MacAppBuilder(args.project_dir)

    try:
        ok = builder.build(skip_build=args.skip_build, open_folder=args.open)
        sys.exit(0 if ok else 1)
    except KeyboardInterrupt:
        print("\n\n🛑 Przerwane przez użytkownika")
        sys.exit(130)
    except Exception as e:
        print(f"\n\n💥 Nieoczekiwany błąd: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)


if __name__ == "__main__":
    main()
