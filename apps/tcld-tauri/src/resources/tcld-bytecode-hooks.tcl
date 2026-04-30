namespace eval ::tcld_bytecode_hooks {
    proc default_hook_commands {} {
        return {
            ::tbcload::bceval
            ::tbcload::bcproc
            ::tbceval
            ::tbcproc
        }
    }

    proc normalize_hook_command_name {command_name} {
        set command_name [string trim $command_name]
        if {$command_name eq ""} {
            return ""
        }
        if {[string match "::*" $command_name]} {
            return $command_name
        }
        return "::$command_name"
    }

    proc normalize_hook_commands {commands} {
        if {[llength $commands] == 0} {
            set commands [default_hook_commands]
        }

        set normalized {}
        foreach command_name $commands {
            set normalized_name [normalize_hook_command_name $command_name]
            if {$normalized_name eq ""} {
                continue
            }
            if {[lsearch -exact $normalized $normalized_name] < 0} {
                lappend normalized $normalized_name
            }
        }
        return $normalized
    }

    proc is_bcproc_command {command} {
        switch -- $command {
            "::tbcload::bcproc" -
            "::tbcproc" {
                return 1
            }
            default {
                return 0
            }
        }
    }

    proc ensure_intercept_loader_shims {} {
        # Intercept mode bypasses package require tbcload and maps legacy
        # tbcload command names onto already-loaded runtime entrypoints.
        namespace eval ::tbcload {}

        if {[llength [info commands ::tbcload::bceval]] == 0} {
            if {[llength [info commands ::tbceval]] == 0} {
                error "intercept mode requires ::tbceval or ::tbcload::bceval"
            }
            proc ::tbcload::bceval {payload} {
                return [::tbceval $payload]
            }
        }

        if {[llength [info commands ::tbcload::bcproc]] == 0} {
            if {[llength [info commands ::tbcproc]] != 0} {
                proc ::tbcload::bcproc {name arglist payload} {
                    return [::tbcproc $name $arglist $payload]
                }
            }
        }
    }
}
