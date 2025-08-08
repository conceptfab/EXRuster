#!/usr/bin/env python3
"""
Automatyczny skrypt kompilacji dla aplikacji rustExR
Autor: Projekt rustExR - EXR File Viewer
"""

import subprocess
import sys
import os
import time
import argparse
from pathlib import Path

class RustBuilder:
    def __init__(self, project_dir="."):
        self.project_dir = Path(project_dir).resolve()
        self.cargo_toml = self.project_dir / "Cargo.toml"
        
    def print_header(self, message):
        """Wyświetla nagłówek z ramką"""
        print("\n" + "="*60)
        print(f"  {message}")
        print("="*60)
        
    def print_step(self, step, message):
        """Wyświetla krok z numerem"""
        print(f"\n[{step}] {message}")
        print("-" * 40)
        
    def check_cargo_project(self):
        """Sprawdza czy to jest prawidłowy projekt Cargo"""
        if not self.cargo_toml.exists():
            print(f"❌ Błąd: Nie znaleziono pliku Cargo.toml w {self.project_dir}")
            print("   Upewnij się, że uruchamiasz skrypt w katalogu projektu Rust.")
            return False
        return True
        
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
                
            return False, e
            
        except Exception as e:
            print(f"❌ Nieoczekiwany błąd: {e}")
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
            # Sprawdź czy plik wykonywalny został utworzony
            exe_name = "rustexr.exe" if os.name == "nt" else "rustexr"
            exe_dir = "release" if release else "debug"
            exe_path = self.project_dir / "target" / exe_dir / exe_name
            
            if exe_path.exists():
                size = exe_path.stat().st_size / (1024 * 1024)  # MB
                print(f"📦 Plik wykonywalny utworzony: {exe_path}")
                print(f"   Rozmiar: {size:.2f} MB")
            else:
                print(f"⚠️  Nie znaleziono pliku wykonywalnego: {exe_path}")
                
        return success
        
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
    
    args = parser.parse_args()
    
    # Tworzenie buildera
    builder = RustBuilder(args.project_dir)
    
    try:
        if args.clean_only:
            # Tylko czyszczenie
            builder.print_header("🗑️  CZYSZCZENIE CACHE KOMPILACJI")
            if not builder.check_cargo_project():
                sys.exit(1)
            success = builder.clean_build()
            sys.exit(0 if success else 1)
            
        elif args.check_only:
            # Tylko sprawdzenie
            builder.print_header("🔍 SPRAWDZANIE SKŁADNI PROJEKTU")
            if not builder.check_cargo_project():
                sys.exit(1)
            builder.clean_build()
            success = builder.check_project()
            sys.exit(0 if success else 1)
            
        else:
            # Pełny proces
            success = builder.full_build_and_run(
                release=not args.debug,  # Domyślnie release, chyba że --debug
                run_tests=args.run_tests,
                example=args.example,
                verbose=args.verbose
            )
            sys.exit(0 if success else 1)
            
    except KeyboardInterrupt:
        print("\n\n🛑 Proces przerwany przez użytkownika")
        sys.exit(130)
    except Exception as e:
        print(f"\n\n💥 Nieoczekiwany błąd: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main()