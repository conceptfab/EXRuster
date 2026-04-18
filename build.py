#!/usr/bin/env python3
"""
Automatyczny skrypt kompilacji dla aplikacji rustExR
Autor: Projekt rustExR - EXR File Viewer
"""

import subprocess
import sys
import os
import re
import time
import argparse
import shutil
from pathlib import Path
from typing import Optional

class RustBuilder:
    def __init__(self, project_dir="."):
        self.project_dir = Path(project_dir).resolve()
        self.cargo_toml = self.project_dir / "Cargo.toml"
    
    def _read_package_version(self) -> Optional[str]:
        """Odczytuje wersję pakietu z Cargo.toml."""
        try:
            content = self.cargo_toml.read_text(encoding="utf-8")
        except Exception:
            return None
        try:
            import tomllib  # type: ignore
            data = tomllib.loads(content)
            pkg = data.get("package", {})
            ver = pkg.get("version")
            return str(ver).strip() if isinstance(ver, str) else None
        except Exception:
            pass
        for line in content.splitlines():
            line = line.strip()
            if line.startswith("version") and "=" in line:
                try:
                    part = line.split("=", 1)[1].strip().strip('"')
                    return part
                except Exception:
                    pass
        return None

    @staticmethod
    def _is_valid_semver(version: str) -> bool:
        """Sprawdza czy wersja pasuje do formatu semver (np. 1.2.3, 1.2.3-alpha.1, 1.2.3+build)."""
        pattern = r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
        return bool(re.match(pattern, version))

    @staticmethod
    def _compare_semver(a: str, b: str) -> int:
        """Porównuje major.minor.patch. Zwraca -1/0/1. Ignoruje suffiksy pre-release/build."""
        def core(v: str) -> tuple:
            base = re.split(r"[-+]", v, 1)[0]
            return tuple(int(x) for x in base.split("."))
        try:
            pa, pb = core(a), core(b)
        except ValueError:
            return 0
        return (pa > pb) - (pa < pb)

    def set_cargo_version(self, new_version: str) -> bool:
        """Aktualizuje pole version w sekcji [package] pliku Cargo.toml.

        Podmienia tylko pierwsze wystąpienie 'version = "..."' w sekcji [package],
        aby nie naruszyć innych sekcji (np. [dependencies]).
        """
        if not self._is_valid_semver(new_version):
            print(f"❌ Niepoprawny format wersji: '{new_version}'")
            print("   💡 Podpowiedź: Użyj semver np. 0.3.6 lub 1.2.3-alpha.1")
            return False
        try:
            content = self.cargo_toml.read_text(encoding="utf-8")
        except Exception as e:
            print(f"❌ Nie można odczytać Cargo.toml: {e}")
            return False

        old_version = self._read_package_version()
        if old_version == new_version:
            print(f"ℹ️  Wersja w Cargo.toml jest już ustawiona na {new_version} — pomijam zapis")
            return True

        lines = content.splitlines(keepends=True)
        in_package = False
        replaced = False
        version_re = re.compile(r'^(\s*version\s*=\s*")([^"]*)(".*)$')
        for idx, raw in enumerate(lines):
            stripped = raw.strip()
            if stripped.startswith("[package]"):
                in_package = True
                continue
            if in_package and stripped.startswith("["):
                break
            if in_package:
                m = version_re.match(raw)
                if m:
                    lines[idx] = f'{m.group(1)}{new_version}{m.group(3)}'
                    if not lines[idx].endswith("\n"):
                        lines[idx] += "\n"
                    replaced = True
                    break
        if not replaced:
            print("❌ Nie znaleziono pola 'version' w sekcji [package] Cargo.toml")
            return False
        try:
            self.cargo_toml.write_text("".join(lines), encoding="utf-8")
        except Exception as e:
            print(f"❌ Nie można zapisać Cargo.toml: {e}")
            return False
        print(f"📝 Zaktualizowano wersję w Cargo.toml: {old_version} → {new_version}")
        print("   ℹ️  Cargo.lock zostanie automatycznie odświeżony podczas 'cargo build'")
        return True

    def read_cargo_lock_version(self, package_name: str) -> Optional[str]:
        """Odczytuje wersję konkretnego pakietu z Cargo.lock.

        Skanuje bloki [[package]] i zwraca wartość 'version' dla pasującej nazwy.
        """
        lock_path = self.project_dir / "Cargo.lock"
        if not lock_path.exists():
            return None
        try:
            content = lock_path.read_text(encoding="utf-8")
        except Exception:
            return None
        try:
            import tomllib  # type: ignore
            data = tomllib.loads(content)
            for pkg in data.get("package", []):
                if pkg.get("name") == package_name:
                    ver = pkg.get("version")
                    if isinstance(ver, str):
                        return ver.strip()
        except Exception:
            pass
        # Fallback – prosty skan liniowy
        name_re = re.compile(r'^\s*name\s*=\s*"([^"]+)"\s*$')
        version_re = re.compile(r'^\s*version\s*=\s*"([^"]+)"\s*$')
        current_name: Optional[str] = None
        for raw in content.splitlines():
            stripped = raw.strip()
            if stripped == "[[package]]":
                current_name = None
                continue
            m_name = name_re.match(raw)
            if m_name:
                current_name = m_name.group(1)
                continue
            m_ver = version_re.match(raw)
            if m_ver and current_name == package_name:
                return m_ver.group(1)
        return None

    def refresh_cargo_lock(self) -> bool:
        """Synchronizuje Cargo.lock z Cargo.toml bez kompilacji (tylko pakiety workspace)."""
        ok, _ = self.run_command(
            ["cargo", "update", "--workspace"],
            "Odświeżanie Cargo.lock (workspace)",
            live_output=False,
        )
        return ok

    def detect_bin_name(self):
        """Wykrywa nazwę binarki na podstawie Cargo.toml.
        Zwraca nazwę lub None, jeśli nie udało się wykryć.
        """
        try:
            content = self.cargo_toml.read_text(encoding="utf-8")
        except Exception:
            return None

        # Najpierw spróbuj użyć tomllib (Python 3.11+)
        try:
            import tomllib  # type: ignore
            data = tomllib.loads(content)
            # Preferuj pierwszą definicję z [[bin]]
            bin_tables = data.get("bin")
            if isinstance(bin_tables, list) and bin_tables:
                name = bin_tables[0].get("name")
                if isinstance(name, str) and name.strip():
                    return name.strip()
            # Fallback do nazwy pakietu
            package = data.get("package", {})
            pkg_name = package.get("name")
            if isinstance(pkg_name, str) and pkg_name.strip():
                return pkg_name.strip()
        except Exception:
            pass

        # Prosty parser liniowy jako fallback
        lines = content.splitlines()
        in_bin = False
        for raw_line in lines:
            line = raw_line.strip()
            if line.startswith("[[bin]]"):
                in_bin = True
                continue
            if in_bin and line.startswith("name") and "=" in line:
                try:
                    name_part = line.split("=", 1)[1].strip()
                    if name_part.startswith('"') and '"' in name_part[1:]:
                        name = name_part.split('"')[1]
                        if name:
                            return name
                except Exception:
                    pass
        # Ostateczny fallback: spróbuj znaleźć name w [package]
        in_package = False
        for raw_line in lines:
            line = raw_line.strip()
            if line.startswith("[package]"):
                in_package = True
                continue
            if in_package:
                if line.startswith("[") and not line.startswith("[package]"):
                    break
                if line.startswith("name") and "=" in line:
                    try:
                        name_part = line.split("=", 1)[1].strip()
                        if name_part.startswith('"') and '"' in name_part[1:]:
                            name = name_part.split('"')[1]
                            if name:
                                return name
                    except Exception:
                        pass
        return None
        
    def print_header(self, message):
        """Wyświetla nagłówek z ramką"""
        print("\n" + "="*60)
        print(f"  {message}")
        print("="*60)
        
    def print_step(self, step, message):
        """Wyświetla krok z numerem"""
        print(f"\n[{step}] {message}")
        print("-" * 40)
        
    def check_rust_environment(self) -> bool:
        """Sprawdza czy cargo i rustc są dostępne w PATH. Wyświetla wersje."""
        print("🔍 Weryfikacja środowiska Rust...")
        ok = True
        for cmd, name in [(["cargo", "--version"], "cargo"), (["rustc", "--version"], "rustc")]:
            try:
                result = subprocess.run(
                    cmd,
                    capture_output=True,
                    text=True,
                    timeout=5,
                    cwd=self.project_dir,
                )
                if result.returncode == 0 and result.stdout.strip():
                    ver = result.stdout.strip().split("\n")[0]
                    print(f"   ✓ {ver}")
                else:
                    print(f"   ❌ {name}: nie znaleziono lub błąd")
                    ok = False
            except (subprocess.TimeoutExpired, FileNotFoundError, OSError) as e:
                print(f"   ❌ {name}: {e}")
                ok = False
        if not ok:
            print("   💡 Podpowiedź: Zainstaluj Rust (rustup) z https://rustup.rs")
        return ok

    def kill_rust_compilation_processes(self) -> list[str]:
        """Sprawdza i ubija działające w tle procesy kompilacji Rust (rustc.exe, cargo.exe).
        Zwraca listę nazw procesów, które zostały zakończone.
        """
        if os.name != "nt":
            # Na systemach Unix można dodać pkill/killall
            return []
        process_names = ["rustc.exe", "cargo.exe"]
        killed = []
        for name in process_names:
            try:
                result = subprocess.run(
                    ["taskkill", "/F", "/IM", name],
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                if result.returncode == 0:
                    killed.append(name)
            except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
                pass
        if killed:
            print(f"🛑 Zakończono procesy kompilacji w tle: {', '.join(killed)}")
            time.sleep(0.5)  # Krótka pauza na zwolnienie blokad
        print("✓ Środowisko gotowe do kompilacji")
        return killed

    def check_cargo_project(self):
        """Sprawdza czy to jest prawidłowy projekt Cargo"""
        if not self.cargo_toml.exists():
            print(f"❌ Błąd: Nie znaleziono pliku Cargo.toml w {self.project_dir}")
            print("   Upewnij się, że uruchamiasz skrypt w katalogu projektu Rust.")
            return False
        return True
        
    def _get_error_hint(self, command: list[str]) -> str:
        """Zwraca podpowiedź w zależności od nieudanej komendy."""
        cmd_str = " ".join(command) if isinstance(command, (list, tuple)) else str(command)
        if "clean" in cmd_str:
            return "Upewnij się, że żadne procesy cargo/rustc nie blokują plików. Uruchom ponownie: python build.py (ubije procesy w tle)"
        if "build" in cmd_str or "check" in cmd_str:
            return "Sprawdź błędy powyżej. Przydatne: cargo check, cargo clippy"
        if "test" in cmd_str:
            return "Sprawdź błędy powyżej. Więcej szczegółów: cargo test -- --nocapture"
        return "Sprawdź logi powyżej i dokumentację projektu."

    def run_command(self, command, description, live_output=False):
        """Uruchamia komendę i zwraca wynik"""
        print(f"🔄 {description}...")
        print(f"   Komenda: {' '.join(command)}")
        
        start_time = time.time()
        
        try:
            if live_output:
                # Live podgląd - nie przechwytuj wyjścia
                result = subprocess.run(
                    command,
                    cwd=self.project_dir,
                    check=True
                )
                elapsed = time.time() - start_time
                print(f"✅ {description} ukończone w {elapsed:.2f}s")
                return True, result
            else:
                # Standardowy tryb z przechwyceniem wyjścia
                result = subprocess.run(
                    command,
                    cwd=self.project_dir,
                    capture_output=True,
                    text=True,
                    check=True
                )
            
            elapsed = time.time() - start_time
            print(f"✅ {description} ukończone w {elapsed:.2f}s")
            
            if result.stdout:
                print("📋 Stdout:")
                print(result.stdout)
                
            return True, result
            
        except subprocess.CalledProcessError as e:
            elapsed = time.time() - start_time
            print(f"❌ {description} nie powiodło się po {elapsed:.2f}s")
            print(f"   Kod błędu: {e.returncode}")
            
            if e.stdout:
                print("📋 Stdout:")
                print(e.stdout)
                
            if e.stderr:
                print("🚨 Stderr:")
                print(e.stderr)

            hint = self._get_error_hint(command)
            print(f"   💡 Podpowiedź: {hint}")
                
            return False, e
            
        except Exception as e:
            print(f"❌ Nieoczekiwany błąd: {e}")
            print(f"   💡 Podpowiedź: Sprawdź czy cargo jest w PATH. Uruchom: cargo --version")
            return False, e
            
    def clean_build(self, verbose=False):
        """Czyści poprzednią kompilację"""
        self.print_step("1", "Czyszczenie poprzedniej kompilacji")
        
        success, result = self.run_command(
            ["cargo", "clean"],
            "Czyszczenie cache kompilacji",
            live_output=verbose
        )
        
        if success:
            # Sprawdź czy folder target został usunięty
            target_dir = self.project_dir / "target"
            if target_dir.exists():
                print(f"⚠️  Folder target nadal istnieje: {target_dir}")
            else:
                print("🗑️  Folder target został wyczyszczony")
                
        return success
        
    def build_project(self, release=True):
        """Kompiluje projekt"""
        mode = "release" if release else "debug"
        self.print_step("2", f"Kompilacja projektu (tryb: {mode})")
        
        command = ["cargo", "build"]
        if release:
            command.append("--release")
            
        success, result = self.run_command(
            command,
            f"Kompilacja w trybie {mode}",
            live_output=True  # Zawsze pokazuj live podgląd dla kompilacji
        )
        
        if success:
            # Sprawdź czy plik wykonywalny został utworzony na podstawie Cargo.toml
            detected_bin = self.detect_bin_name()
            exe_dir = "release" if release else "debug"
            if detected_bin:
                exe_name = f"{detected_bin}.exe" if os.name == "nt" else detected_bin
                exe_path = self.project_dir / "target" / exe_dir / exe_name
                if exe_path.exists():
                    size = exe_path.stat().st_size / (1024 * 1024)  # MB
                    print(f"📦 Plik wykonywalny utworzony: {exe_path}")
                    print(f"   Rozmiar: {size:.2f} MB")
                else:
                    print(f"⚠️  Nie znaleziono spodziewanego pliku wykonywalnego: {exe_path}")
            else:
                print("⚠️  Nie udało się wykryć nazwy binarki z Cargo.toml – pomijam sprawdzenie artefaktu.")
                
        return success

    def prompt_for_version(self, current_version: Optional[str]) -> Optional[str]:
        """Interaktywnie pyta o nowy numer wersji. ENTER = zachowaj aktualną."""
        current_str = current_version or "(nieznana)"
        print("\n" + "="*60)
        print(f"  🔖 NUMER WERSJI")
        print("="*60)
        print(f"   Aktualna wersja w Cargo.toml: {current_str}")
        print(f"   Podaj nowy numer semver (np. 0.3.6) lub ENTER, aby zachować aktualną.")
        while True:
            try:
                response = input("   Nowa wersja: ").strip()
            except EOFError:
                return None
            if not response:
                print(f"   ↪ Zachowuję aktualną wersję: {current_str}")
                return None
            if self._is_valid_semver(response):
                return response
            print(f"   ❌ Niepoprawny format '{response}'. Użyj semver (np. 0.3.6 lub 1.2.3-alpha.1).")

    def build_final(self, bin_name: str = "EXruster", out_name: str = "EXruster", out_dir: str = "dist", clean: bool = False, verbose: bool = False, jobs: Optional[int] = None, set_version: Optional[str] = None, force_downgrade: bool = False) -> bool:
        """Buduje finalną wersję binarki w trybie release i kopiuje do katalogu out_dir bez uruchamiania."""
        self.print_header("🚀 FINALNY BUILD APLIKACJI")
        print(f"📁 Katalog projektu: {self.project_dir}")
        print(f"🦀 Tryb kompilacji: release")
        print(f"🔧 Binarka (Cargo): {bin_name}")
        print(f"📦 Docelowa nazwa pliku: {out_name}")
        print(f"📤 Katalog wyjściowy: {out_dir}")

        if not self.check_cargo_project():
            return False

        if set_version is None:
            set_version = self.prompt_for_version(self._read_package_version())

        if set_version:
            print(f"🔖 Nowa wersja do ustawienia: {set_version}")

        # Stan rollbacku: jeśli zmieniamy wersję, zachowaj oryginalne pliki
        cargo_toml_backup: Optional[str] = None
        success = False

        try:
            # Opcjonalnie: zmiana wersji w Cargo.toml przed kompilacją + fail-fast weryfikacja
            if set_version:
                self.print_step("0a", "Aktualizacja wersji w Cargo.toml")
                old_version = self._read_package_version()

                # Ochrona przed downgrade'm
                if old_version and self._compare_semver(set_version, old_version) < 0:
                    if force_downgrade:
                        print(f"⚠️  Downgrade {old_version} → {set_version} wymuszony przez --force-downgrade")
                    else:
                        print(f"❌ Próba downgrade'u: {old_version} → {set_version}")
                        print("   💡 Podpowiedź: Jeśli to zamierzone, dodaj flagę --force-downgrade")
                        return False

                # Backup aktualnego Cargo.toml przed zapisem
                try:
                    cargo_toml_backup = self.cargo_toml.read_text(encoding="utf-8")
                except Exception as e:
                    print(f"❌ Nie można odczytać Cargo.toml do backupu: {e}")
                    return False

                if not self.set_cargo_version(set_version):
                    return False

                # Fail-fast: odśwież i zweryfikuj Cargo.lock zanim ruszymy z ciężką kompilacją
                self.print_step("0b", "Odświeżanie i weryfikacja Cargo.lock")
                if not self.refresh_cargo_lock():
                    print("❌ Nie udało się odświeżyć Cargo.lock")
                    return False
                lock_version = self.read_cargo_lock_version(bin_name)
                if lock_version != set_version:
                    print(f"❌ Cargo.lock NIE pasuje do oczekiwanej wersji po odświeżeniu!")
                    print(f"   Oczekiwano: {set_version}")
                    print(f"   W Cargo.lock: {lock_version or '(brak wpisu)'}")
                    return False
                print(f"🔐 Cargo.lock zweryfikowany: {bin_name} = {lock_version}")

            # Zawsze spróbuj oczyścić katalog target przed finalnym buildem
            self.print_step("0", "Czyszczenie katalogu 'target'")
            cleaned_ok = self.clean_build(verbose=verbose)
            if not cleaned_ok:
                # Fallback: spróbuj ręcznie usunąć folder target (zignoruj błędy)
                target_dir = self.project_dir / "target"
                try:
                    if target_dir.exists():
                        print(f"⚠️  cargo clean nie powiodło się – próba usunięcia: {target_dir}")
                        shutil.rmtree(target_dir, ignore_errors=True)
                        if target_dir.exists():
                            print("⚠️  Nie udało się w pełni usunąć folderu 'target' (możliwe zablokowane pliki)")
                        else:
                            print("🗑️  Folder 'target' usunięty (fallback)")
                except Exception as e:
                    print(f"⚠️  Fallback usunięcia 'target' nie powiódł się: {e}")

            # Build release konkretnej binarki (build.rs doda datę kompilacji do wersji)
            self.print_step("1", f"Kompilacja binarki '{bin_name}' w trybie release")
            cmd = ["cargo", "build", "--release", "--bin", bin_name]
            if jobs is not None:
                cmd.extend(["-j", str(jobs)])
            ok, _ = self.run_command(cmd, f"Kompilacja '{bin_name}' (release)", live_output=True)
            if not ok:
                return False

            # Ścieżki artefaktów (zbudowany plik ma nazwę binarki z Cargo)
            target_dir = self.project_dir / "target" / "release"
            built_exe = f"{bin_name}.exe" if os.name == "nt" else bin_name
            built_path = target_dir / built_exe
            if not built_path.exists():
                print(f"❌ Nie znaleziono skompilowanego pliku: {built_path}")
                print(f"   💡 Podpowiedź: Sprawdź nazwę binarki (--bin {bin_name}) w Cargo.toml")
                return False

            # Przygotuj katalog wyjściowy (docelowa nazwa bez _nightly)
            out_path = self.project_dir / out_dir
            out_path.mkdir(parents=True, exist_ok=True)
            final_exe = f"{out_name}.exe" if os.name == "nt" else out_name
            final_path = out_path / final_exe

            try:
                shutil.copy2(built_path, final_path)
            except Exception as e:
                print(f"❌ Kopiowanie do {final_path} nie powiodło się: {e}")
                print(f"   💡 Podpowiedź: Sprawdź uprawnienia i czy plik nie jest zablokowany przez inną aplikację")
                return False

            size_mb = final_path.stat().st_size / (1024 * 1024)
            # Odczytaj wersję z Cargo.toml (build.rs dodał datę wewnątrz exe)
            pkg_version = self._read_package_version()
            build_datetime = time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime())
            version_str = f"{pkg_version} ({build_datetime})" if pkg_version else "?"
            print(f"\n✅ Finalny plik: {final_path}")
            print(f"   Rozmiar: {size_mb:.2f} MB")
            print(f"   Wersja: {version_str}")
            success = True
            return True
        finally:
            # Rollback Cargo.toml jeśli cokolwiek po bumpie wersji poszło nie tak
            # (KeyboardInterrupt również tu trafia — finally wykona się przed propagacją)
            if not success and cargo_toml_backup is not None:
                try:
                    self.cargo_toml.write_text(cargo_toml_backup, encoding="utf-8")
                    print(f"↩️  Przywrócono oryginalny Cargo.toml (rollback po błędzie)")
                    print(f"   ℹ️  Cargo.lock zostanie zsynchronizowany przy następnym wywołaniu cargo")
                except Exception as e:
                    print(f"⚠️  Nie udało się przywrócić Cargo.toml: {e}")
                    print(f"   💡 Ręcznie przywróć wersję w [package] w Cargo.toml")
        
    def check_project(self):
        """Sprawdza projekt bez kompilacji"""
        self.print_step("2", "Sprawdzanie składni i typów")
        
        success, result = self.run_command(
            ["cargo", "check"],
            "Sprawdzanie składni"
        )
        
        return success
        
    def run_tests(self):
        """Uruchamia testy"""
        self.print_step("3", "Uruchamianie testów")
        
        success, result = self.run_command(
            ["cargo", "test"],
            "Uruchamianie testów jednostkowych"
        )
        
        return success
        
    def run_application(self, example=None, release=True):
        """Uruchamia aplikację"""
        if example:
            self.print_step("4", f"Uruchamianie przykładu: {example}")
            command = ["cargo", "run", "--example", example]
            description = f"Uruchamianie przykładu {example}"
        else:
            self.print_step("4", "Uruchamianie głównej aplikacji")
            command = ["cargo", "run"]
            description = "Uruchamianie głównej aplikacji"
            
        # Dodaj flagę release jeśli potrzebna
        if release:
            command.append("--release")
            
        print(f"🚀 {description}...")
        print(f"   Komenda: {' '.join(command)}")
        print("   (Naciśnij Ctrl+C aby zatrzymać aplikację)")
        
        try:
            # Uruchom aplikację bez przechwytywania wyjścia
            subprocess.run(
                command,
                cwd=self.project_dir,
                check=True
            )
        except KeyboardInterrupt:
            print("\n🛑 Aplikacja zatrzymana przez użytkownika")
        except subprocess.CalledProcessError as e:
            print(f"\n❌ Aplikacja zakończyła się błędem (kod: {e.returncode})")
            
    def full_build_and_run(self, release=True, run_tests=False, example=None, verbose=False):
        """Pełny proces: czyszczenie, kompilacja i uruchomienie"""
        self.print_header("🔨 AUTOMATYCZNA KOMPILACJA PROJEKTU RUSTEXR")
        
        if not self.check_cargo_project():
            return False
            
        print(f"📁 Katalog projektu: {self.project_dir}")
        print(f"🦀 Tryb kompilacji: {'release' if release else 'debug'}")
        print(f"🧪 Testy: {'tak' if run_tests else 'nie'}")
        if example:
            print(f"📝 Przykład: {example}")
            
        # Krok 1: Czyszczenie
        if not self.clean_build(verbose=verbose):
            print("\n❌ Proces przerwany na etapie czyszczenia")
            return False
            
        # Krok 2: Kompilacja
        if not self.build_project(release):
            print("\n❌ Proces przerwany na etapie kompilacji")
            return False
            
        # Krok 3: Testy (opcjonalnie)
        if run_tests:
            if not self.run_tests():
                print("\n⚠️  Testy nie przeszły, ale kontynuujemy...")
                
        # Krok 4: Uruchomienie
        print("\n🎯 Kompilacja zakończona pomyślnie!")
        
        response = input("\n❓ Czy chcesz uruchomić aplikację? (t/n): ").strip().lower()
        if response in ['t', 'tak', 'y', 'yes']:
            self.run_application(example, release=release)
            
        return True


def main():
    parser = argparse.ArgumentParser(
        description="Automatyczny skrypt kompilacji dla projektu rustExR (Rust/Slint)",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Przykłady użycia:
  python build.py                    # Standardowa kompilacja RELEASE
  python build.py --debug            # Kompilacja debug
  python build.py --verbose          # Kompilacja z pełnym podglądem
  python build.py --check-only       # Tylko sprawdzenie składni
  python build.py --run-tests        # Kompilacja z testami
  python build.py --example simple   # Kompilacja i uruchomienie przykładu
  python build.py --clean-only       # Tylko czyszczenie
  python build.py --set-version 0.3.6  # Zmień wersję i zbuduj finalny artefakt
        """
    )
    
    parser.add_argument(
        "--debug", 
        action="store_true",
        help="Kompiluj w trybie debug (domyślnie: release)"
    )
    
    parser.add_argument(
        "--verbose", 
        action="store_true",
        help="Pokaż szczegółowe wyjście podczas kompilacji"
    )
    
    parser.add_argument(
        "--check-only",
        action="store_true", 
        help="Tylko sprawdź składnię, nie kompiluj"
    )
    
    parser.add_argument(
        "--clean-only",
        action="store_true",
        help="Tylko wyczyść cache kompilacji"
    )

    parser.add_argument(
        "--clean",
        action="store_true",
        help="Wykonaj cargo clean przed finalnym buildem (domyślnie: nie)"
    )
    
    parser.add_argument(
        "--run-tests",
        action="store_true",
        help="Uruchom testy po kompilacji"
    )
    
    parser.add_argument(
        "--example",
        type=str,
        help="Uruchom konkretny przykład zamiast głównej aplikacji"
    )
    
    parser.add_argument(
        "--project-dir",
        type=str,
        default=".",
        help="Ścieżka do katalogu projektu (domyślnie: bieżący katalog)"
    )

    parser.add_argument(
        "--bin",
        type=str,
        default="EXruster",
        help="Nazwa binarki Cargo do zbudowania (domyślnie: EXruster)"
    )

    parser.add_argument(
        "--out-name",
        type=str,
        default="EXruster",
        help="Docelowa nazwa pliku wyjściowego bez rozszerzenia (domyślnie: EXruster)"
    )

    parser.add_argument(
        "--out-dir",
        type=str,
        default="dist",
        help="Katalog docelowy dla finalnego pliku (domyślnie: dist)"
    )

    parser.add_argument(
        "--jobs",
        type=int,
        default=None,
        metavar="N",
        help="Liczba zadań cargo (np. 1 przy LNK1104)"
    )

    parser.add_argument(
        "--set-version",
        type=str,
        default=None,
        metavar="X.Y.Z",
        help="Zmień wersję w [package] Cargo.toml przed kompilacją (semver). Cargo.lock zostanie odświeżony i zweryfikowany przed buildem."
    )

    parser.add_argument(
        "--force-downgrade",
        action="store_true",
        help="Pozwól na zmianę wersji w dół (domyślnie blokowane przez ochronę przed downgrade)"
    )

    args = parser.parse_args()
    
    # Tworzenie buildera
    builder = RustBuilder(args.project_dir)
    
    try:
        # Weryfikacja środowiska (cargo, rustc)
        if not builder.check_rust_environment():
            print("\n❌ Środowisko Rust nie jest gotowe. Zainstaluj rustup i spróbuj ponownie.")
            sys.exit(1)

        # Ubij ewentualne procesy kompilacji Rust działające w tle
        builder.kill_rust_compilation_processes()

        start_time = time.time()

        if args.clean_only:
            # Tylko czyszczenie
            builder.print_header("🗑️  CZYSZCZENIE CACHE KOMPILACJI")
            if not builder.check_cargo_project():
                print(f"\n⏱️  Proces przerwany po {time.time() - start_time:.1f}s")
                sys.exit(1)
            success = builder.clean_build()
            elapsed = time.time() - start_time
            print(f"\n⏱️  Cały proces zakończony w {elapsed:.1f}s")
            sys.exit(0 if success else 1)
            
        elif args.check_only:
            # Tylko sprawdzenie
            builder.print_header("🔍 SPRAWDZANIE SKŁADNI PROJEKTU")
            if not builder.check_cargo_project():
                print(f"\n⏱️  Proces przerwany po {time.time() - start_time:.1f}s")
                sys.exit(1)
            builder.clean_build()
            success = builder.check_project()
            elapsed = time.time() - start_time
            print(f"\n⏱️  Cały proces zakończony w {elapsed:.1f}s")
            sys.exit(0 if success else 1)
            
        else:
            # Domyślne zachowanie: zbuduj finalny artefakt bez uruchamiania
            builder.print_header("BUDOWANIE FINALNEGO ARTEFAKTU")
            if not builder.check_cargo_project():
                print(f"\n⏱️  Proces przerwany po {time.time() - start_time:.1f}s")
                sys.exit(1)
            success = builder.build_final(bin_name=args.bin, out_name=args.out_name, out_dir=args.out_dir, clean=args.clean, verbose=args.verbose, jobs=args.jobs, set_version=args.set_version, force_downgrade=args.force_downgrade)
            elapsed = time.time() - start_time
            print(f"\n⏱️  Cały proces zakończony w {elapsed:.1f}s")
            sys.exit(0 if success else 1)
            
    except KeyboardInterrupt:
        print("\n\n🛑 Proces przerwany przez użytkownika")
        sys.exit(130)
    except Exception as e:
        print(f"\n\n💥 Nieoczekiwany błąd: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()