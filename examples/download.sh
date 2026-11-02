# Sourced by the examples: download_archive NAME URL unzips URL in examples/downloads/NAME the
# first time, then reuses it; it prints the folder. With curl or wget, and unzip or Python.
# The archives are their authors' and are not part of bemu: they are fetched from their sites.

download_archive() {
    downloads="$(cd "$(dirname "$0")" && pwd)/downloads"
    folder="$downloads/$1"
    if [ ! -d "$folder" ]; then
        mkdir -p "$downloads"
        archive="$downloads/$1.zip"
        echo "downloading $2 (once, into $folder)" >&2
        if command -v curl >/dev/null; then
            curl -fL --progress-bar -o "$archive.part" "$2"
        else
            wget -q --show-progress -O "$archive.part" "$2"
        fi
        mv "$archive.part" "$archive"
        rm -rf "$folder.part"
        mkdir "$folder.part"
        if command -v unzip >/dev/null; then
            unzip -q "$archive" -d "$folder.part"
        else
            python3 -I -m zipfile -e "$archive" "$folder.part"
        fi
        mv "$folder.part" "$folder"
        rm -f "$archive"
    fi
    echo "$folder"
}
