#!/bin/sh
# Writes the Microsoft Store listing import file (installer/store/store-listing.csv) from
# the listing texts in installer/store/store-listing-fr-FR.md and store-listing-en-US.md.
# Used by `make store-listing`; import the CSV in Partner Center (Store listings >
# Import), from installer/store/ where the images it names are.
#
#   sh scripts/store-listing.sh
#
# In the .md files, each "## ..." section fills a field: the description keeps its
# paragraphs, and the features and search terms are one per line. A line in parentheses
# is a note, left out. The "- Label: value" lines give the copyright, the developer, the
# Store logo and the screenshot (file name only).
set -eu

dir=installer/store
out=$dir/store-listing.csv

awk '
# The French file comes first, then the English one: the CSV columns.
FNR == 1 { lang++; section = "" }

/^## / { section = substr($0, 4); next }
/^# / || /^\(.*\)$/ { next }

/^- [^:]+: / {
    label = substr($0, 3, index($0, ": ") - 3)
    value = substr($0, index($0, ": ") + 2)
    if (label == "Copyright and trademark info") set("Copyright", value)
    else if (label == "Developed by") set("DevelopedBy", value)
    else if (label == "Store logo") set("StoreLogos1", value)
    else if (label == "Screenshot") { sub(/.*\//, "", value); set("Screenshots1", value) }
    next
}

section == "Product features" { item("ProductFeatures", 20); next }
section == "Search terms" { item("SearchTerms", 7); next }

section == "Product name" { text("ProductName"); next }
section == "Short description" { text("ShortDescription"); next }
section == "Description" { text("Description"); next }
section == "What'"'"'s new in this version" { text("WhatsNew"); next }
section == "License terms" { text("Applicable license terms"); next }

function set(field, value) { val[lang, field] = value }

# One line per item; blank lines are skipped.
function item(prefix, max) {
    if ($0 == "") return
    n = ++count[lang, prefix]
    if (n > max) {
        printf "%s: more than %d lines under \"%s\"\n", FILENAME, max, section > "/dev/stderr"
        failed = 1
        exit 1
    }
    set(prefix n, $0)
}

# Lines kept as they are, blank ones between paragraphs too, but not at the start or end.
function text(field) {
    if ($0 == "") { if ((lang, field) in val) blank[lang, field]++; return }
    if ((lang, field) in val) {
        sep = "\n"
        for (i = 0; i < blank[lang, field]; i++) sep = sep "\n"
        val[lang, field] = val[lang, field] sep $0
    } else val[lang, field] = $0
    blank[lang, field] = 0
}

function csv(s) {
    if (s ~ /[",\n]/) { gsub(/"/, "\"\"", s); s = "\"" s "\"" }
    return s
}

function row(field, type) {
    print csv(field) "," csv(type) "," csv(val[1, field]) "," csv(val[2, field])
}

END {
    if (failed) exit 1
    if (lang != 2) { print "expected the French and English files" > "/dev/stderr"; exit 1 }
    image = "Provide the file name only if it is in the same folder. Provide relative path to image starting with the subfolder name otherwise"
    printf "\357\273\277"
    print "Field,Type,French,English"
    row("ProductName", "Text")
    row("Description", "Text")
    row("WhatsNew", "Text")
    for (i = 1; i <= 20; i++) row("ProductFeatures" i, "Text")
    row("ShortDescription", "Text")
    for (i = 1; i <= 7; i++) row("SearchTerms" i, "Text")
    row("Applicable license terms", "Text")
    row("Copyright", "Text")
    row("DevelopedBy", "Text")
    for (i = 1; i <= 11; i++) row("RequirementsMinimum" i, "Text")
    for (i = 1; i <= 11; i++) row("RequirementsRecommended" i, "Text")
    for (i = 1; i <= 2; i++) row("StoreLogos" i, image)
    for (i = 1; i <= 10; i++) row("Screenshots" i, image)
    row("HeroArts", "Text")
    row("Trailers", "Text")
}
' "$dir/store-listing-fr-FR.md" "$dir/store-listing-en-US.md" > "$out.tmp"
mv "$out.tmp" "$out"
echo "$out"
