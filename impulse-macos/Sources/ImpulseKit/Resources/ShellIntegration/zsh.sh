# Impulse shell integration for zsh
zmodload zsh/parameter 2>/dev/null
__impulse_command_started=""
__impulse_names_sig=""
__impulse_path_sent=""
__impulse_urlencode() {
    local string="$1" i c
    local encoded=""
    for (( i=0; i<${#string}; i++ )); do
        c="${string:$i:1}"
        case "$c" in
            [a-zA-Z0-9._~/-]) encoded+="$c" ;;
            *) printf -v encoded "%s%%%02X" "$encoded" "'$c" ;;
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
    printf '\e]6973;Command=%s\a' "$(__impulse_urlencode "$command")"
    printf '\e]133;C\a'
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd __impulse_precmd
add-zsh-hook preexec __impulse_preexec
