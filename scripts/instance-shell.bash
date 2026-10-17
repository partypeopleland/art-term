# Scoped to one App work Shell. Preserve normal user initialization first.
if [[ -f ~/.bashrc ]]; then source ~/.bashrc; fi
# Keep existing aliases/functions: they can intentionally bypass this integration.
art-term-screen() {
    python3 "$ART_TERM_SCREEN_BRIDGE" shell-command --binding "$ART_TERM_SCREEN_BINDING" "$@"
}
if ! alias codex >/dev/null 2>&1 && ! declare -F codex >/dev/null; then
    codex() {
        python3 "$ART_TERM_SCREEN_BRIDGE" launch --binding "$ART_TERM_SCREEN_BINDING" -- codex "$@"
    }
fi
