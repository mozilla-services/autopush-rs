#! python3

# If we are running on trixie+ we need to add a few env vars
# to the build. Bash does not make this easy. So for now
# we'll use this ugly hack script to check the version,
# append the right flags, and run whatever you ask us to.
import sys
import os

RUN = " ".join(sys.argv[1:])
PREFIX = ""

try:
    # The version file is either a numeric (e.g. "12.3") or
    # a codename (e.g."bookworm/sid")
    # Since AFAIK, this only impact debian, skip if we don't have a debian version file.
    with open("/etc/debian_version") as f:
        version = f.read().strip().lower()
        try:
            # First try to parse as numeric
            major_version = int(version.split(".")[0])
        except ValueError:
            # That didn't work, so try codename.
            major_version = version.split("/")[0].lower()
            match major_version:
                case "bookworm":
                    major_version = 12
                case "bullseye":
                    major_version = 11
                case "buster":
                    major_version = 10
                case "stretch":
                    major_version = 9
                case "jessie":
                    major_version = 8
                case _:
                    # Honestly, we don't care if it's older than bookworm.
                    major_version = 13
        if major_version > 12:
            os.environ["CXXFLAGS"] = "-include cstdint"
            os.environ["CMAKE_POLICY_VERSION_MINIMUM"] = "3.5"
            PREFIX = f"""CXXFLAGS='-include cstdint' CMAKE_POLICY_VERSION_MINIMUM=3.5"""
except Exception as e:
    print(f"Error: {e}", file=sys.stderr)

print(f"""{PREFIX} {RUN}""")
os.system(f"""{PREFIX} {RUN}""")
