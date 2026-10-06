# Impulse shell integration for bash
__impulse_command_started=""
# Set while PROMPT_COMMAND runs: the DEBUG trap fires for its entries
# (history -a, direnv, starship…) too, and those aren't commands.
__impulse_in_prompt=""
__impulse_names_sent=""
__impulse_path_sent=""
__impulse_urlencode() {
    # Byte by byte in the C locale, so non-ASCII text is sent as its UTF-8
    # bytes (bash 3.2 reports bytes above 0x7F as negative numbers).
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
    __impulse_in_prompt=1
    # Keep the end marker last, after anything added to PROMPT_COMMAND since.
    case "$PROMPT_COMMAND" in
        *__impulse_prompt_end) ;;
        *) PROMPT_COMMAND="${PROMPT_COMMAND//;__impulse_prompt_end/};__impulse_prompt_end" ;;
    esac
    if [ -n "$__impulse_command_started" ]; then
        printf '\e]133;D;%d\a' "$exit_code"
        __impulse_command_started=""
    fi
    printf '\e]7;file://%s%s\a' "$HOSTNAME" "$(__impulse_urlencode "$PWD")"
    __impulse_report_names
    printf '\e]133;A\a'
}
__impulse_prompt_end() {
    __impulse_in_prompt=""
}
__impulse_preexec() {
    local command="$1"
    [ -n "$__impulse_in_prompt" ] && return
    case "$command" in
        __impulse_prompt_command*|__impulse_prompt_end*|__impulse_preexec*) return ;;
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
    PROMPT_COMMAND="__impulse_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND};__impulse_prompt_end"
fi
__impulse_orig_debug_trap=$(trap -p DEBUG | sed "s/trap -- '\\(.*\\)' DEBUG/\\1/")
trap '__impulse_preexec "$BASH_COMMAND"; eval "$__impulse_orig_debug_trap"' DEBUG
