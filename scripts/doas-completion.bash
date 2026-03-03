
# doas wrapper: delegate "doas chvm ..." completions to chvm
_doas_chvm() {
    if [[ $COMP_CWORD -ge 2 && "${COMP_WORDS[1]}" == "chvm" ]]; then
        COMP_WORDS=("${COMP_WORDS[@]:1}")
        COMP_CWORD=$((COMP_CWORD - 1))
        _clap_complete_chvm chvm "$2" "${COMP_WORDS[COMP_CWORD-1]}"
    fi
}
complete -o nospace -o bashdefault -o nosort -F _doas_chvm doas
