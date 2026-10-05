# Impulse shell integration for bash
__impulse_command_started=""
__impulse_names_sent=""
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
# Commands that aren't files on PATH (aliases, functions, builtins, keywords)
# and PATH itself, for the input bar's unknown-command underline. Names go
# again after a command that may define some.
__impulse_report_names() {
    if [ -z "$__impulse_names_sent" ]; then
        __impulse_names_sent=1
        local names
        names=$(compgen -a -A function -b -k 2>/dev/null)
        names="${names//[^[:graph:]]/ }"
        printf '\e]6973;Names=%s\a' "${names:0:60000}"
    fi
    if [ "$PATH" != "$__impulse_path_sent" ]; then
        __impulse_path_sent="$PATH"
        printf '\e]6973;Path=%s\a' "${PATH//[^[:graph:] ]/}"
    fi
}
__impulse_prompt_command() {
    local exit_code=$?
    if [ -n "$__impulse_command_started" ]; then
        printf '\e]133;D;%d\a' "$exit_code"
        __impulse_command_started=""
    fi
    printf '\e]7;file://%s%s\a' "$HOSTNAME" "$(__impulse_urlencode "$PWD")"
    __impulse_report_names
    printf '\e]133;A\a'
}
__impulse_preexec() {
    local command="$1"
    case "$command" in
        __impulse_prompt_command*|__impulse_preexec*) return ;;
    esac
    if [ -n "$__impulse_command_started" ]; then
        return
    fi
    __impulse_command_started=1
    case "$command" in
        alias*|unalias*|source*|". "*|function*|*"()"*|unset*|eval*) __impulse_names_sent="" ;;
    esac
    printf '\e]6973;Command=%s\a' "$(__impulse_urlencode "$command")"
    printf '\e]133;C\a'
}
if [[ ! "$PROMPT_COMMAND" == *"__impulse_prompt_command"* ]]; then
    PROMPT_COMMAND="__impulse_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
__impulse_orig_debug_trap=$(trap -p DEBUG | sed "s/trap -- '\\(.*\\)' DEBUG/\\1/")
trap '__impulse_preexec "$BASH_COMMAND"; eval "$__impulse_orig_debug_trap"' DEBUG
