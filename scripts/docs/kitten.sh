#!/bin/sh
# Stand-in for `kitten icat` in Impulse's documentation screenshots,
# installed as ~/bin/kitten in the demo home: sends each PNG named by the
# kitty graphics protocol in the escape sequences themselves (t=d, base64
# in pieces of 4096 bytes), shown at the cursor, then a newline, as icat
# does without --place. Unlike icat it shows each over 72 columns and 12
# rows (c=72,r=12), so a shot is the same in points at any scale and with
# any font, and it asks the terminal nothing.
#
#   kitten icat FILE…

[ "$1" = icat ] || { echo "kitten: only icat is stood in for" >&2; exit 1; }
shift
# The file's base64 a piece a line (the last one ended too, for `read`).
pieces() { base64 < "$1" | tr -d '\n' | fold -w 4096; echo; }
for file in "$@"; do
    count=$(pieces "$file" | wc -l)
    pieces "$file" | {
        n=0
        while IFS= read -r piece; do
            n=$((n + 1))
            more=$([ "$n" -lt "$count" ] && echo 1 || echo 0)
            if [ "$n" -eq 1 ]; then
                printf '\033_Ga=T,f=100,c=72,r=12,q=2,m=%s;%s\033\\' "$more" "$piece"
            else
                printf '\033_Gm=%s;%s\033\\' "$more" "$piece"
            fi
        done
    }
    printf '\n'
done
