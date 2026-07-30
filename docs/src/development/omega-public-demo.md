# Capture the Omega public demo (removed)

The public demo workroom launch path was removed with the full-editor mode
split (omega#161). `--demo-workroom` staged a fictional workroom inside the
legacy editor surface, and it required the deleted editor flag; both flags are
gone, and `script/omega-public-demo` was deleted with them.

The fixture data remains in the repository at `assets/demo-workroom/`, and the
workroom UI's fixed in-memory demo projection remains in `crates/workroom_ui`.
A future demo capture path, if one is wanted, must be designed against the one
shipped surface rather than the deleted editor layout.

Historical: the launch invocation and capture checklist for the old demo are
in this file's Git history.
