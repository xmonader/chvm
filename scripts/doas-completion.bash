
# doas wrapper: delegate "doas chvm ..." completions to chvm
_doas_chvm() {
    if [[ $COMP_CWORD -ge 2 && "${COMP_WORDS[1]}" == "chvm" ]]; then
        # Lazy-load chvm completions if not yet sourced
        if ! declare -f _clap_complete_chvm &>/dev/null; then
            local comp_file="${XDG_DATA_HOME:-$HOME/.local/share}/bash-completion/completions/chvm"
            [[ -f "$comp_file" ]] && source "$comp_file"
        fi
        COMP_WORDS=("${COMP_WORDS[@]:1}")
        COMP_CWORD=$((COMP_CWORD - 1))
        _clap_complete_chvm chvm "$2" "${COMP_WORDS[COMP_CWORD-1]}"
    fi
}
complete -o nospace -o bashdefault -o nosort -F _doas_chvm doas
