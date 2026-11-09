# Write or remove form `/testpath` marker assets (step 9).
# Must stay aligned with `registration_server::update::{upgrade_probe_testpath_html,upgrade_probe_js}`.
#
# Usage: write_upgrade_probe_testpath <form-dir> [letter]
# Empty / invalid letter removes testpath.html and upgrade-probe.js (shipping).

write_upgrade_probe_testpath() {
  local form_dir=$1
  local raw=${2:-}
  local letter
  letter=$(printf '%s' "$raw" | tr -d '[:space:]')
  mkdir -p "$form_dir"
  rm -f "$form_dir/testpath.html" "$form_dir/upgrade-probe.js"
  if [[ ! "$letter" =~ ^[A-Za-z]$ ]]; then
    return 0
  fi
  letter=$(printf '%s' "$letter" | tr '[:lower:]' '[:upper:]')
  cat >"$form_dir/upgrade-probe.js" <<EOF
export const UPGRADE_PROBE = "${letter}";
EOF
  cat >"$form_dir/testpath.html" <<EOF
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Upgrade probe</title>
</head>
<body>
<h1>Upgrade probe</h1>
<p class="probe-letter">${letter}</p>
<script type="module" src="./upgrade-probe.js"></script>
</body>
</html>
EOF
}
