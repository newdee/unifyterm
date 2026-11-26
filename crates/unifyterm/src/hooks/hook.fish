# Written by unifyterm each time it starts: changes here are lost.
# fish runs this after its own start files, in unifyterm's tabs only.
# `[history] enabled = false` in unifyterm's settings turns it off.
#
# It tells unifyterm, in standard terminal marks, where the prompt starts
# and ends (OSC 133;A and B), when a command starts (133;C), what it was
# and how it ended (633;E, 133;D) and which directory this is (633;P;Cwd).
# Adapted from keepane's shell hooks (MIT, same author).

# unifyterm's commands first in PATH again: fish's start files may have put
# other directories in front.
if set -q UNIFYTERM_COMMANDS
    set -l __unifyterm_rest
    for __unifyterm_entry in $PATH
        test "$__unifyterm_entry" = "$UNIFYTERM_COMMANDS"; or set -a __unifyterm_rest $__unifyterm_entry
    end
    set -gx PATH $UNIFYTERM_COMMANDS $__unifyterm_rest
    set -e __unifyterm_entry
end

function __unifyterm_escape
    string replace -a '\\' '\\\\' -- $argv[1] | string replace -a ';' '\\x3b' | string join '\\x0a'
end

function __unifyterm_preexec --on-event fish_preexec
    # Two printfs: in fish's single quotes \\ is one backslash, and one in the middle of a
    # format would take the character after it (it printed "e]133;C\" on the screen).
    printf '\e]633;E;%s\e\\' (__unifyterm_escape "$argv[1]")
    printf '\e]133;C\e\\'
end

function __unifyterm_postexec --on-event fish_postexec
    printf '\e]133;D;%s\e\\' $status
    set -g __unifyterm_ran 1
end

function __unifyterm_cwd --on-event fish_prompt
    printf '\e]633;P;Cwd=%s\e\\' (__unifyterm_escape "$PWD")
end

if set -q UNIFYTERM_PROMPT
    # With prompt.enabled in unifyterm's settings, ut prompt writes the prompt; the part after
    # \x1e goes on the right.
    function fish_prompt
        set -l code $status
        set -l arguments prompt --shell fish --cwd $PWD --width $COLUMNS
        if set -q __unifyterm_ran
            set -a arguments --status $code --took $CMD_DURATION
            set -e __unifyterm_ran
        end
        set -l out ("$UNIFYTERM_PROMPT" $arguments 2>/dev/null | string collect)
        # Each mark printed by itself: in fish's single quotes \\ is one backslash, which
        # would take the character after it (%s) away from printf.
        printf '\e]133;A\e\\'
        if test -z "$out"
            printf '> '
        else
            set -l parts (string split \x1e -- $out)
            set -g __unifyterm_right $parts[2]
            printf '%s' $parts[1]
        end
        printf '\e]133;B\e\\'
    end
    function fish_right_prompt
        printf '%s' $__unifyterm_right
    end
else if functions -q fish_prompt
    functions -c fish_prompt __unifyterm_user_prompt
    function fish_prompt
        printf '\e]133;A\e\\'
        __unifyterm_user_prompt
        printf '\e]133;B\e\\'
    end
end

printf '\e]633;P;Shell=fish\e\\'
