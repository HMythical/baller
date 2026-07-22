#!/usr/bin/env python3
"""
Repository Structure Validation Script

Validates that required directories exist in the repository.
Used by GitHub Actions to ensure proper project structure.
"""

import os
import sys
from pathlib import Path


REQUIRED_DIRECTORIES = ["src", "build"]


def validate_structure(repo_root: Path) -> bool:
    """
    Validate that required directories exist in the repository.
    
    Args:
        repo_root: Path to the repository root directory
        
    Returns:
        True if all required directories exist, False otherwise
    """
    print("Checking required directories...")
    print()
    
    missing = []
    
    for dir_name in REQUIRED_DIRECTORIES:
        dir_path = repo_root / dir_name
        if dir_path.exists() and dir_path.is_dir():
            print(f"✓ Found required directory: {dir_name}")
        else:
            missing.append(dir_name)
            print(f"✗ Missing required directory: {dir_name}")
    
    print()
    
    if missing:
        print(f"ERROR: Required directories missing: {', '.join(missing)}")
        return False
    
    print("✓ All required directories exist")
    return True


def main() -> int:
    """Main entry point for the script."""
    # Get repository root from environment or current directory
    repo_root_str = os.environ.get("GITHUB_WORKSPACE", os.getcwd())
    repo_root = Path(repo_root_str)
    
    print("=" * 50)
    print("Repository Structure Validation")
    print("=" * 50)
    print()
    
    success = validate_structure(repo_root)
    
    print()
    print("=" * 50)
    
    if success:
        print("Result: PASSED")
    else:
        print("Result: FAILED")
    
    print("=" * 50)
    
    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())
