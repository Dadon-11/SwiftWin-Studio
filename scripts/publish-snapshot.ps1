# Make a clean, history-free copy of SwiftWin to publish (docs/publishing.md).
#
#   powershell -ExecutionPolicy Bypass -File scripts\publish-snapshot.ps1 -Dest ..\swiftwin-public `
#       -AuthorName "Your Name or handle" -AuthorEmail "12345+you@users.noreply.github.com"
#
# Copies only files tracked in git at HEAD (so untracked Hello\, MyApp\, target\ and git-ignored files
# such as swiftwin-build.env never come along), drops folders that are private notes, scans the copy
# for personal data and keys, and only then makes ONE commit in a NEW repository. This repository and
# its history are not changed. Nothing is pushed.
#
# Your own words to look for (your name, email, usernames, phone name...) go in publish-patterns.txt
# next to this repo's root, one regular expression per line. That file is git-ignored, so the
# patterns themselves are never published.
param(
    [Parameter(Mandatory = $true)][string]$Dest,
    [Parameter(Mandatory = $true)][string]$AuthorName,
    [Parameter(Mandatory = $true)][string]$AuthorEmail,
    [switch]$Force
)
$ErrorActionPreference = "Stop"
$root = (git rev-parse --show-toplevel).Trim()
if (Test-Path $Dest) {
    if (-not $Force) { throw "$Dest exists. Pick a new folder (or -Force to replace it)." }
    Remove-Item -Recurse -Force $Dest
}
New-Item -ItemType Directory -Force $Dest | Out-Null
$Dest = (Resolve-Path $Dest).Path
if ($Dest.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw "Put the copy outside the repository." }

# 1. Tracked files only.
$zip = Join-Path ([IO.Path]::GetTempPath()) ("swiftwin-snapshot-" + [guid]::NewGuid() + ".zip")
git -C $root archive --format=zip -o $zip HEAD
Expand-Archive -Path $zip -DestinationPath $Dest
Remove-Item $zip

# 2. Private notes that don't belong in the public repo.
foreach ($p in @("docs\history", "publish-patterns.txt", "swiftwin-build.env")) {
    $f = Join-Path $Dest $p
    if (Test-Path $f) { Remove-Item -Recurse -Force $f; Write-Host "removed $p" }
}

# Private claude.ai links (e.g. the design doc in CLAUDE.md): drop those lines from the copy.
Get-ChildItem -Recurse -File $Dest -Include *.md | ForEach-Object {
    $lines = Get-Content $_.FullName
    $kept = $lines | Where-Object { $_ -notmatch 'claude\.ai/(code/)?artifact/' }
    if ($kept.Count -ne $lines.Count) { Set-Content -Path $_.FullName -Value $kept -Encoding utf8; Write-Host "removed a private claude.ai link from $($_.Name)" }
}

# 3. Scan: keys and tokens always; your own patterns; the IDs from swiftwin-build.env.
$patterns = @(
    'sb_secret_[A-Za-z0-9_-]{16,}', 'sb_publishable_[A-Za-z0-9_-]{16,}', 'eyJ[A-Za-z0-9_-]{20,}\.eyJ[A-Za-z0-9_-]{20,}\.',
    '-----BEGIN [A-Z ]*PRIVATE KEY-----\s*[A-Za-z0-9+/]{40,}', 'gh[pousr]_[A-Za-z0-9]{30,}', 'github_pat_[A-Za-z0-9_]{20,}',
    'AKIA[0-9A-Z]{16}', 'sk-ant-[A-Za-z0-9-]{20,}', 'xox[bp]-[A-Za-z0-9-]{10,}',
    '[a-z]{20}\.supabase\.co', 'Ov23[A-Za-z0-9]{16}', 'claude\.ai/(code/)?artifact/'
)
$own = Join-Path $root "publish-patterns.txt"
if (Test-Path $own) { $patterns += Get-Content $own | Where-Object { $_.Trim() -and -not $_.StartsWith("#") } }
else { Write-Warning "No publish-patterns.txt: only generic key patterns are checked (add your name, email, usernames)." }
$ids = Join-Path $root "swiftwin-build.env"
if (Test-Path $ids) {
    foreach ($l in Get-Content $ids) { if ($l -match '^\s*[A-Z_]+\s*=\s*(.+?)\s*$') { $patterns += [regex]::Escape($Matches[1]) } }
}
$hits = Get-ChildItem -Recurse -File $Dest | Where-Object { $_.Length -lt 5MB -and $_.Name -ne "publish-snapshot.ps1" } |
    Select-String -Pattern $patterns -AllMatches -ErrorAction SilentlyContinue
if ($hits) {
    $hits | ForEach-Object { "{0}:{1}: {2}" -f $_.Path.Substring($Dest.Length + 1), $_.LineNumber, $_.Line.Trim() } | Select-Object -First 50
    throw "Found $($hits.Count) lines that look personal or secret (above). Fix them in the repo, commit, and run this again."
}
Write-Host "scan: clean ($($patterns.Count) patterns)"
Write-Host "Images are not scanned: look at every .png/.jpg under $Dest yourself."

# 4. A new repository with a single commit.
git -C $Dest init -q -b main
git -C $Dest -c user.name="$AuthorName" -c user.email="$AuthorEmail" add -A
git -C $Dest -c user.name="$AuthorName" -c user.email="$AuthorEmail" commit -q -m "SwiftWin: first public release"
git -C $Dest config user.name "$AuthorName"
git -C $Dest config user.email "$AuthorEmail"
git -C $Dest log --format="%h %an <%ae> %s"
Write-Host "Done: $Dest. Nothing was pushed. Review it, then follow docs/publishing.md."
