# Impulse shell integration for zsh
zmodload zsh/parameter 2>/dev/null
# The terminal's secret for the command text this reports, so what programs
# print can't pass for a command. A shell variable, not exported: programs
# run here don't see it.
__impulse_nonce="${__impulse_nonce:-${IMPULSE_SHELL_NONCE:-}}"
unset IMPULSE_SHELL_NONCE
__impulse_command_started=""
__impulse_names_sig=""
__impulse_path_sent=""
__impulse_urlencode() {
    # Byte by byte, so non-ASCII text is sent as its UTF-8 bytes rather than
    # as code points.
    emulate -L zsh
    setopt no_multibyte
    local LC_ALL=C
    local string="$1" i c v
    local encoded=""
    for (( i=0; i<${#string}; i++ )); do
        c="${string:$i:1}"
        case "$c" in
            [a-zA-Z0-9._~/-]) encoded+="$c" ;;
            *)
                printf -v v '%d' "'$c"
                (( v < 0 )) && (( v += 256 ))
                printf -v encoded '%s%%%02X' "$encoded" "$v"
                ;;
        esac
    done
    printf '%s' "$encoded"
}
# Commands that aren't files on PATH (aliases, functions, builtins, reserved
# words) and PATH itself, for the input bar's unknown-command underline. Sent
# again only when they change.
__impulse_report_names() {
    local sig="${#aliases}:${#functions}"
    if [[ "$sig" != "$__impulse_names_sig" ]]; then
        __impulse_names_sig="$sig"
        local names="${(k)aliases} ${(k)functions} ${(k)builtins} ${(k)reswords}"
        names="${names//[^[:graph:] ]/}"
        printf '\e]6973;Names=%s\a' "${names[1,60000]}"
    fi
    if [[ "$PATH" != "$__impulse_path_sent" ]]; then
        __impulse_path_sent="$PATH"
        printf '\e]6973;Path=%s\a' "${PATH//[^[:graph:] ]/}"
    fi
}
__impulse_precmd() {
    local exit_code=$?
    if [ -n "$__impulse_command_started" ]; then
        printf '\e]133;D;%d\a' "$exit_code"
        __impulse_command_started=""
    fi
    printf '\e]7;file://%s%s\a' "$HOST" "$(__impulse_urlencode "$PWD")"
    __impulse_report_names
    printf '\e]133;A\a'
}
__impulse_preexec() {
    local command="$1"
    __impulse_command_started=1
    printf '\e]6973;Command=%s;Nonce=%s\a' "$(__impulse_urlencode "$command")" "$__impulse_nonce"
    printf '\e]133;C\a'
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd __impulse_precmd
add-zsh-hook preexec __impulse_preexec
