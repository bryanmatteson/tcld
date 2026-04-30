#!/usr/bin/env tclsh
# tcld-agent.tcl — Debug agent injected into tclsh by tcld
#
# Usage: tclsh tcld-agent.tcl --port <port> [--bytecode-hook-mode passthrough|intercept]
#                              [--bytecode-hook-command <command>]...
#                              [--debug-extension-path <path>] --script <target.tcl> [args...]
#
# Protocol: JSON-over-TCP, newline-delimited.
# Each message: {"type":"...","data":{...}}\n

package require Tcl 8.5

namespace eval ::tcld_agent {
    # ── State ──────────────────────────────────────────────────────────
    variable sock ""
    variable state "idle"          ;# idle | running | paused
    variable step_mode ""          ;# "" | step_into | step_over | step_out
    variable step_depth 0          ;# frame depth at time of step command
    variable paused_depth 0        ;# frame depth where execution is currently paused
    variable breakpoints {}        ;# list of dicts: {file line condition}
    variable target_script ""
    variable agent_script [info script]
    variable target_args {}
    variable port 0
    variable read_buf ""
    variable chunk_seq 0
    variable control_mode "continue"
    variable command_hooks [dict create]
    variable suppress_trace 0          ;# re-entrancy guard for debug_handler
    variable execution_trace_commands {}
    variable trace_decision_ready 0
    variable trace_decision_pause 0
    variable trace_decision_reason ""
    variable bytecode_hook_mode "passthrough"
    variable bytecode_hook_commands {}
    variable debug_extension_path ""
    variable debug_extension_loaded 0
    variable debug_extension_installed 0
    variable debug_extension_capabilities ""
    variable debug_extension_reason "disabled"

    # ── JSON encoding (minimal, no external deps) ──────────────────────
    proc json_string {s} {
        set result "\""
        set len [string length $s]
        for {set i 0} {$i < $len} {incr i} {
            set ch [string index $s $i]
            switch -- $ch {
                "\\" { append result "\\\\" }
                "\"" { append result "\\\"" }
                "\n" { append result "\\n" }
                "\r" { append result "\\r" }
                "\t" { append result "\\t" }
                "\b" { append result "\\b" }
                "\f" { append result "\\f" }
                default {
                    scan $ch %c code
                    if {$code < 0x20} {
                        append result [format "\\u%04x" $code]
                    } else {
                        append result $ch
                    }
                }
            }
        }
        append result "\""
        return $result
    }

    proc json_dict {pairs} {
        set parts {}
        foreach {k v} $pairs {
            lappend parts "[json_string $k]:$v"
        }
        return "\{[join $parts ,]\}"
    }

    proc json_list {items} {
        return "\[[join $items ,]\]"
    }

    proc json_bool {val} {
        if {$val} { return "true" } else { return "false" }
    }

    proc json_null {} { return "null" }

    proc json_int {val} { return $val }

    # ── Debug logging to stderr ──────────────────────────────────────
    proc _debug_log {msg} {
        catch {
            set fd stderr
            if {[info commands ::tcld_agent::_original_puts] ne ""} {
                ::tcld_agent::_original_puts $fd "\[tcld-agent\] $msg"
            } else {
                puts $fd "\[tcld-agent\] $msg"
            }
        }
    }

    proc ensure_bytecode_hook_support_loaded {} {
        if {[llength [info commands ::tcld_bytecode_hooks::default_hook_commands]] > 0} {
            return
        }

        set hooks_file [file join [file dirname [info script]] tcld-bytecode-hooks.tcl]
        if {![file exists $hooks_file]} {
            error "missing bytecode hook support file: $hooks_file"
        }
        if {[catch {source $hooks_file} err]} {
            error "failed to source $hooks_file: $err"
        }
    }

    proc ensure_debug_extension_support_loaded {} {
        if {[llength [info commands ::tcld_debug_extension::load_and_install]] > 0} {
            return
        }

        set support_file [file join [file dirname [info script]] tcld-debug-extension.tcl]
        if {![file exists $support_file]} {
            _debug_log "debug extension support file missing: $support_file"
            return
        }
        if {[catch {source $support_file} err]} {
            _debug_log "failed to source debug extension support: $err"
        }
    }

    proc maybe_load_debug_extension {} {
        variable debug_extension_path
        variable debug_extension_loaded
        variable debug_extension_installed
        variable debug_extension_capabilities
        variable debug_extension_reason

        set debug_extension_loaded 0
        set debug_extension_installed 0
        set debug_extension_capabilities ""
        set debug_extension_reason "disabled"

        set path [string trim $debug_extension_path]
        if {$path eq ""} {
            return
        }

        ensure_debug_extension_support_loaded
        if {[llength [info commands ::tcld_debug_extension::load_and_install]] == 0} {
            set debug_extension_reason "support_missing"
            return
        }

        if {[catch {set result [::tcld_debug_extension::load_and_install $path]} err]} {
            _debug_log "debug extension load error: $err"
            set debug_extension_reason "load_failed"
            return
        }

        if {[dict exists $result loaded]} {
            set debug_extension_loaded [expr {[dict get $result loaded] ? 1 : 0}]
        }
        if {[dict exists $result installed]} {
            set debug_extension_installed [expr {[dict get $result installed] ? 1 : 0}]
        }
        if {[dict exists $result capabilities]} {
            set debug_extension_capabilities [dict get $result capabilities]
        }
        if {[dict exists $result reason]} {
            set debug_extension_reason [dict get $result reason]
        }

        if {!$debug_extension_loaded} {
            _debug_log "debug extension not loaded: [dict get $result reason]"
        }
    }

    # ── Protocol: send message to bridge ──────────────────────────────
    proc send_msg {type data} {
        variable sock
        if {$sock eq ""} return
        set msg [json_dict [list type [json_string $type] data $data]]
        if {[catch {
            puts $sock $msg
            flush $sock
        } err]} {
            teardown
        }
    }

    # ── Protocol: parse incoming JSON (simple recursive descent) ──────
    # We only need to parse commands from the bridge, which are simple
    # key-value objects. No need for a full JSON parser.

    proc parse_json {str} {
        set str [string trim $str]
        if {[string index $str 0] eq "\{"} {
            return [parse_json_object $str]
        } elseif {[string index $str 0] eq "\["} {
            return [parse_json_array $str]
        } elseif {[string index $str 0] eq "\""} {
            return [parse_json_string $str]
        } elseif {$str eq "true"} {
            return 1
        } elseif {$str eq "false"} {
            return 0
        } elseif {$str eq "null"} {
            return ""
        } else {
            # number
            return $str
        }
    }

    proc parse_json_string {str} {
        set inner [string range $str 1 end-1]
        set result ""
        set len [string length $inner]
        for {set i 0} {$i < $len} {incr i} {
            set ch [string index $inner $i]
            if {$ch eq "\\"} {
                incr i
                if {$i >= $len} { append result "\\"; break }
                set esc [string index $inner $i]
                switch -- $esc {
                    "\"" { append result "\"" }
                    "\\" { append result "\\" }
                    "/"  { append result "/" }
                    "n"  { append result "\n" }
                    "r"  { append result "\r" }
                    "t"  { append result "\t" }
                    "b"  { append result "\b" }
                    "f"  { append result "\f" }
                    "u"  {
                        set hex [string range $inner [expr {$i+1}] [expr {$i+4}]]
                        incr i 4
                        scan $hex %x code
                        append result [format %c $code]
                    }
                    default { append result $esc }
                }
            } else {
                append result $ch
            }
        }
        return $result
    }

    proc parse_json_object {str} {
        set str [string trim $str]
        set str [string range $str 1 end-1] ;# strip { }
        set result [dict create]
        set str [string trim $str]
        if {$str eq ""} { return $result }

        # Tokenize respecting nested structures and strings
        set pairs [json_split_comma $str]
        foreach pair $pairs {
            set pair [string trim $pair]
            set colon [json_find_colon $pair]
            if {$colon < 0} continue
            set key [string trim [string range $pair 0 $colon-1]]
            set val [string trim [string range $pair $colon+1 end]]
            set key [parse_json $key]
            set val [parse_json $val]
            dict set result $key $val
        }
        return $result
    }

    proc parse_json_array {str} {
        set str [string trim $str]
        set str [string range $str 1 end-1] ;# strip [ ]
        set str [string trim $str]
        if {$str eq ""} { return {} }
        set items [json_split_comma $str]
        set result {}
        foreach item $items {
            lappend result [parse_json [string trim $item]]
        }
        return $result
    }

    proc json_split_comma {str} {
        set result {}
        set depth 0
        set in_string 0
        set escape 0
        set start 0
        set len [string length $str]

        for {set i 0} {$i < $len} {incr i} {
            set ch [string index $str $i]
            if {$escape} {
                set escape 0
                continue
            }
            if {$ch eq "\\"} {
                set escape 1
                continue
            }
            if {$in_string} {
                if {$ch eq "\""} { set in_string 0 }
                continue
            }
            if {$ch eq "\""} {
                set in_string 1
            } elseif {$ch eq "\{" || $ch eq "\["} {
                incr depth
            } elseif {$ch eq "\}" || $ch eq "\]"} {
                incr depth -1
            } elseif {$ch eq "," && $depth == 0} {
                lappend result [string range $str $start $i-1]
                set start [expr {$i + 1}]
            }
        }
        lappend result [string range $str $start end]
        return $result
    }

    proc json_find_colon {str} {
        set depth 0
        set in_string 0
        set escape 0
        set len [string length $str]

        for {set i 0} {$i < $len} {incr i} {
            set ch [string index $str $i]
            if {$escape} {
                set escape 0
                continue
            }
            if {$ch eq "\\"} {
                set escape 1
                continue
            }
            if {$in_string} {
                if {$ch eq "\""} { set in_string 0 }
                continue
            }
            if {$ch eq "\""} {
                set in_string 1
            } elseif {$ch eq "\{" || $ch eq "\["} {
                incr depth
            } elseif {$ch eq "\}" || $ch eq "\]"} {
                incr depth -1
            } elseif {$ch eq ":" && $depth == 0} {
                return $i
            }
        }
        return -1
    }

    # ── Variable inspection ────────────────────────────────────────────
    proc get_variables {level {depth 1}} {
        set vars {}

        # Locals at the given level
        if {[catch {
            set local_names [uplevel #$level {info locals}]
        }]} {
            set local_names {}
        }

        foreach name [lsort $local_names] {
            if {[catch {
                set is_array [uplevel #$level [list array exists $name]]
            }]} {
                set is_array 0
            }
            if {$is_array} {
                if {[catch {
                    set arr_data [uplevel #$level [list array get $name]]
                }]} {
                    set arr_data {}
                }
                set children {}
                foreach {k v} $arr_data {
                    lappend children [json_dict [list \
                        name [json_string "$name\($k\)"] \
                        value [json_string $v] \
                        type [json_string [detect_type $v]] \
                        scope [json_string "local"] \
                    ]]
                }
                lappend vars [json_dict [list \
                    name [json_string $name] \
                    value [json_string "(array, [expr {[llength $arr_data] / 2}] elements)"] \
                    type [json_string "array"] \
                    scope [json_string "local"] \
                    children [json_list $children] \
                ]]
            } else {
                if {[catch {
                    set val [uplevel #$level [list set $name]]
                }]} {
                    set val "<undefined>"
                }
                lappend vars [json_dict [list \
                    name [json_string $name] \
                    value [json_string $val] \
                    type [json_string [detect_type $val]] \
                    scope [json_string "local"] \
                ]]
            }
        }

        return [json_list $vars]
    }

    proc detect_type {val} {
        # Check if integer
        if {[string is integer -strict $val]} { return "int" }
        # Check if double
        if {[string is double -strict $val]} { return "double" }
        # Check if boolean
        if {[string is boolean -strict $val]} { return "bool" }
        # Check if valid list with >1 element
        if {![catch {llength $val} len] && $len > 1} {
            # Check if dict-like (even number of elements, unique keys)
            if {$len % 2 == 0 && $len >= 2} {
                set keys {}
                set is_dict 1
                foreach {k v} $val {
                    if {$k in $keys} { set is_dict 0; break }
                    lappend keys $k
                }
                if {$is_dict} { return "dict" }
            }
            return "list"
        }
        return "string"
    }

    # ── Stack inspection ───────────────────────────────────────────────
    proc get_stack {} {
        set frames {}
        set depth [info frame]
        # Skip our own frames (agent internals)
        # Walk from the outermost frame inward
        for {set i 1} {$i <= $depth} {incr i} {
            if {[catch {set finfo [info frame $i]} err]} continue

            set type [dict get $finfo type]
            # Skip our own namespace frames
            if {[dict exists $finfo proc]} {
                set pname [dict get $finfo proc]
                if {[string match "::tcld_agent::*" $pname]} continue
            }

            set frame_data {}
            lappend frame_data level [json_int $i]
            lappend frame_data type [json_string $type]

            if {[dict exists $finfo proc]} {
                lappend frame_data proc [json_string [dict get $finfo proc]]
            } else {
                lappend frame_data proc [json_null]
            }
            if {[dict exists $finfo file]} {
                lappend frame_data file [json_string [dict get $finfo file]]
            } else {
                lappend frame_data file [json_null]
            }
            if {[dict exists $finfo line]} {
                lappend frame_data line [json_int [dict get $finfo line]]
            } else {
                lappend frame_data line [json_null]
            }
            if {[dict exists $finfo cmd]} {
                set cmd [dict get $finfo cmd]
                # Truncate long commands
                if {[string length $cmd] > 200} {
                    set cmd "[string range $cmd 0 197]..."
                }
                lappend frame_data cmd [json_string $cmd]
            } else {
                lappend frame_data cmd [json_null]
            }

            lappend frames [json_dict $frame_data]
        }
        return [json_list $frames]
    }

    # ── Eval in context ────────────────────────────────────────────────
    proc eval_at_level {script level} {
        if {[catch {
            set result [uplevel #$level $script]
        } err opts]} {
            set error_info ""
            if {[dict exists $opts -errorinfo]} {
                set error_info [dict get $opts -errorinfo]
            }
            set error_code ""
            if {[dict exists $opts -errorcode]} {
                set error_code [dict get $opts -errorcode]
            }
            return [json_dict [list \
                result [json_string $err] \
                error [json_bool 1] \
                errorInfo [json_string $error_info] \
                errorCode [json_string $error_code] \
            ]]
        }
        return [json_dict [list \
            result [json_string $result] \
            error [json_bool 0] \
        ]]
    }

    # ── Breakpoint matching ────────────────────────────────────────────
    proc check_breakpoint {file line} {
        variable breakpoints
        set file_norm [normalize_file_path $file]
        foreach bp $breakpoints {
            if {[dict exists $bp enabled] && ![dict get $bp enabled]} {
                continue
            }
            set bp_file [dict get $bp file]
            set bp_norm [normalize_file_path $bp_file]
            set bp_line [dict get $bp line]
            # Match by filename (not full path) or full path
            if {($bp_norm eq $file_norm || [file tail $bp_norm] eq [file tail $file_norm]) && $bp_line == $line} {
                # Check condition if any
                if {[dict exists $bp condition] && [dict get $bp condition] ne ""} {
                    set cond [dict get $bp condition]
                    if {[catch {set result [uplevel 1 [list expr $cond]]}]} {
                        return 0
                    }
                    return $result
                }
                return 1
            }
        }
        return 0
    }

    # ── Command hook system ────────────────────────────────────────────
    # Hook proc signature:
    #   hook phase command argv code result options
    # where phase is "before" or "after".
    proc register_command_hook {phase command hook_proc} {
        variable command_hooks
        if {$phase ne "before" && $phase ne "after"} {
            error "invalid hook phase: $phase"
        }
        if {![dict exists $command_hooks $command]} {
            dict set command_hooks $command [dict create before {} after {}]
        }
        set hooks [dict get $command_hooks $command $phase]
        if {[lsearch -exact $hooks $hook_proc] < 0} {
            dict lappend command_hooks $command $phase $hook_proc
        }
    }

    proc run_command_hooks {phase command argv code result options} {
        variable command_hooks
        if {![dict exists $command_hooks $command]} return
        if {![dict exists $command_hooks $command $phase]} return
        foreach hook [dict get $command_hooks $command $phase] {
            catch {uplevel #0 [list $hook $phase $command $argv $code $result $options]}
        }
    }

    proc ensure_command_hook_wrappers {} {
        variable command_hooks
        foreach cmd [dict keys $command_hooks] {
            if {[llength [info commands $cmd]] == 0} {
                continue
            }

            set alias "::tcld_agent::__orig_[string map {: _} $cmd]"
            if {[llength [info commands $alias]] > 0} {
                continue
            }

            rename $cmd $alias
            set body [string map [list __CMD__ $cmd __ALIAS__ $alias] {
                catch {::tcld_agent::run_command_hooks "before" "__CMD__" $args 0 "" {}}
                set __tcld_result ""
                set __tcld_options {}
                set __tcld_code [catch {uplevel 1 [list __ALIAS__ {*}$args]} __tcld_result __tcld_options]
                catch {::tcld_agent::run_command_hooks "after" "__CMD__" $args $__tcld_code $__tcld_result $__tcld_options}
                if {$__tcld_code == 0} {
                    return $__tcld_result
                }
                return -options $__tcld_options $__tcld_result
            }]
            proc $cmd {args} $body
        }
    }

    # ── Bytecode streaming hooks (tbcload::bceval / tbcload::bcproc) ──
    proc default_bytecode_hook_commands {} {
        return [::tcld_bytecode_hooks::default_hook_commands]
    }

    proc normalize_hook_command_name {command_name} {
        return [::tcld_bytecode_hooks::normalize_hook_command_name $command_name]
    }

    proc normalize_bytecode_hook_commands {commands} {
        return [::tcld_bytecode_hooks::normalize_hook_commands $commands]
    }

    proc set_bytecode_hook_mode {mode} {
        variable bytecode_hook_mode

        set mode [string tolower [string trim $mode]]
        if {$mode eq ""} {
            set mode "passthrough"
        }

        switch -- $mode {
            passthrough -
            intercept {
                set bytecode_hook_mode $mode
            }
            default {
                error "invalid --bytecode-hook-mode '$mode' (expected passthrough or intercept)"
            }
        }
    }

    proc is_hooked_bcproc_command {command} {
        return [::tcld_bytecode_hooks::is_bcproc_command $command]
    }

    proc ensure_intercept_loader_shims {} {
        return [::tcld_bytecode_hooks::ensure_intercept_loader_shims]
    }

    proc maybe_intercept_tbcload_require {args} {
        variable bytecode_hook_mode
        if {$bytecode_hook_mode ne "intercept"} {
            return [list 0 ""]
        }
        if {[llength $args] < 2} {
            return [list 0 ""]
        }
        if {[lindex $args 0] ne "require"} {
            return [list 0 ""]
        }
        if {[string tolower [lindex $args 1]] ne "tbcload"} {
            return [list 0 ""]
        }

        if {[catch {::tcld_agent::ensure_intercept_loader_shims} err]} {
            _debug_log "intercept mode shim install failed: $err"
            return [list 0 ""]
        }

        set version "1.6"
        if {[llength $args] >= 3} {
            set requested [string trim [lindex $args 2]]
            if {$requested ne ""} {
                set version $requested
            }
        }
        return [list 1 $version]
    }

    proc emit_bytecode_chunk {command payload {proc_name ""}} {
        variable chunk_seq
        incr chunk_seq

        set payload_size [string length $payload]
        set disassembly ""
        if {$payload ne ""} {
            catch {
                set disassembly [::tcl::unsupported::disassemble script $payload]
            }
        }

        if {$proc_name eq ""} {
            set proc_json [json_null]
        } else {
            set proc_json [json_string $proc_name]
        }

        send_msg "bytecode_chunk" [json_dict [list \
            sequence [json_int $chunk_seq] \
            command [json_string $command] \
            procName $proc_json \
            payload [json_string $payload] \
            payloadSize [json_int $payload_size] \
            disassembly [json_string $disassembly] \
        ]]
    }

    proc emit_command_error {command argv code result options} {
        variable control_mode

        set error_info ""
        set error_code ""
        if {[dict exists $options -errorinfo]} {
            set error_info [dict get $options -errorinfo]
        }
        if {[dict exists $options -errorcode]} {
            set error_code [dict get $options -errorcode]
        }

        send_msg "command_error" [json_dict [list \
            command [json_string $command] \
            controlMode [json_string $control_mode] \
            code [json_int $code] \
            message [json_string $result] \
            errorInfo [json_string $error_info] \
            errorCode [json_string $error_code] \
            argc [json_int [llength $argv]] \
        ]]
    }

    proc stream_bytecode_hook {phase command argv code result options} {
        if {$phase ne "before"} return

        set proc_name ""
        set payload ""

        if {[is_hooked_bcproc_command $command]} {
            if {[llength $argv] >= 1} {
                set proc_name [lindex $argv 0]
            }
            if {[llength $argv] >= 3} {
                set payload [lindex $argv 2]
            }
        } else {
            if {[llength $argv] >= 1} {
                set payload [lindex $argv 0]
            }
        }

        if {$payload eq ""} return
        catch {::tcld_agent::emit_bytecode_chunk $command $payload $proc_name}
    }

    proc command_error_hook {phase command argv code result options} {
        if {$phase ne "after"} return
        if {$code != 1} return
        catch {::tcld_agent::emit_command_error $command $argv $code $result $options}
    }

    proc setup_bytecode_stream_hooks {} {
        variable bytecode_hook_mode
        variable bytecode_hook_commands

        set bytecode_hook_commands [normalize_bytecode_hook_commands $bytecode_hook_commands]

        if {$bytecode_hook_mode eq "intercept"} {
            catch {::tcld_agent::ensure_intercept_loader_shims}
        }

        foreach cmd $bytecode_hook_commands {
            register_command_hook before $cmd ::tcld_agent::stream_bytecode_hook
            register_command_hook after $cmd ::tcld_agent::command_error_hook
        }

        if {[llength [info commands ::tcld_agent::_original_package]] == 0} {
            rename ::package ::tcld_agent::_original_package
            proc ::package {args} {
                set intercept [::tcld_agent::maybe_intercept_tbcload_require $args]
                set handled [lindex $intercept 0]
                if {$handled} {
                    catch {::tcld_agent::ensure_command_hook_wrappers}
                    return [lindex $intercept 1]
                }

                set result [uplevel 1 [list ::tcld_agent::_original_package {*}$args]]
                if {[llength $args] >= 2 && [lindex $args 0] eq "require"} {
                    catch {::tcld_agent::ensure_command_hook_wrappers}
                }
                return $result
            }
        }
        catch {::tcld_agent::ensure_command_hook_wrappers}
    }

    # ── Poll control commands while running ────────────────────────────
    # Running mode normally does not sit in command_loop, so we perform a
    # non-blocking socket poll at trace points to handle pause/step commands.
    proc poll_commands_nonblocking {} {
        variable sock
        variable state

        if {$sock eq ""} return

        set prev_blocking 1
        if {![catch {set prev_blocking [fconfigure $sock -blocking]}]} {
            catch {fconfigure $sock -blocking 0}
        }

        while {$sock ne ""} {
            if {[catch {set n [gets $sock line]} err]} {
                teardown
                break
            }
            if {$n < 0} {
                if {[eof $sock]} {
                    teardown
                }
                break
            }

            set line [string trim $line]
            if {$line eq ""} continue

            if {[catch {set msg [parse_json_object $line]}]} {
                continue
            }

            handle_command $msg
            if {$state ne "running"} {
                break
            }
        }

        if {$sock ne ""} {
            catch {fconfigure $sock -blocking $prev_blocking}
        }
    }

    proc normalize_file_path {path} {
        if {$path eq ""} { return "" }
        if {[catch {file normalize $path} norm]} {
            return $path
        }
        return $norm
    }

    proc add_execution_trace {command_name} {
        variable execution_trace_commands
        if {$command_name eq ""} return
        if {[lsearch -exact $execution_trace_commands $command_name] >= 0} return
        if {[catch {trace add execution $command_name enterstep ::tcld_agent::debug_handler}]} {
            return
        }
        lappend execution_trace_commands $command_name
    }

    proc clear_execution_traces {} {
        variable execution_trace_commands
        foreach command_name $execution_trace_commands {
            catch {trace remove execution $command_name enterstep ::tcld_agent::debug_handler}
        }
        set execution_trace_commands {}
    }

    proc select_debug_location_frame {} {
        variable target_script
        variable agent_script

        set depth [info frame]
        set target_norm [normalize_file_path $target_script]
        set agent_norm [normalize_file_path $agent_script]
        set fallback ""

        for {set i 1} {$i <= $depth} {incr i} {
            if {[catch {set finfo [info frame $i]}]} continue

            if {[dict exists $finfo proc]} {
                set pname [dict get $finfo proc]
                if {[string match "::tcld_agent::*" $pname]} continue
            }

            if {$fallback eq ""} {
                set fallback $finfo
            }

            if {![dict exists $finfo file]} continue
            set file [dict get $finfo file]
            if {$file eq ""} continue

            set file_norm [normalize_file_path $file]
            if {$target_norm ne "" && $file_norm eq $target_norm} {
                return $finfo
            }
            if {$file_norm ne $agent_norm} {
                return $finfo
            }
        }

        return $fallback
    }

    proc wait_for_trace_decision {} {
        variable sock
        variable state
        variable trace_decision_ready

        set deadline [expr {[clock milliseconds] + 5000}]
        while {!$trace_decision_ready && $sock ne "" && $state eq "running"} {
            if {[clock milliseconds] > $deadline} {
                _debug_log "wait_for_trace_decision: timed out, defaulting to continue"
                set trace_decision_ready 1
                set ::tcld_agent::trace_decision_pause 0
                set ::tcld_agent::trace_decision_reason ""
                break
            }

            if {[catch {
                if {[gets $sock line] < 0} {
                    if {[eof $sock]} {
                        teardown
                        return
                    }
                    after 5
                    continue
                }
            } err]} {
                _debug_log "wait_for_trace_decision: read error: $err"
                teardown
                return
            }

            set line [string trim $line]
            if {$line eq ""} continue
            if {[catch {set msg [parse_json_object $line]} err]} {
                _debug_log "wait_for_trace_decision: JSON parse error: $err"
                continue
            }
            if {[catch {handle_command $msg} err]} {
                _debug_log "wait_for_trace_decision: handle_command error: $err"
            }
        }
    }

    # ── Debug handler (trace callback) ────────────────────────────────
    proc debug_handler {args} {
        variable sock
        variable state
        variable paused_depth
        variable suppress_trace
        variable trace_decision_ready
        variable trace_decision_pause
        variable trace_decision_reason

        if {$suppress_trace} return
        if {$sock eq ""} return
        if {$state ne "running"} return

        # Accept queued control commands even while running (e.g. pause).
        set suppress_trace 1
        set _poll_code [catch {poll_commands_nonblocking} _poll_err]
        set suppress_trace 0
        if {$_poll_code != 0} {
            _debug_log "debug_handler: poll_commands_nonblocking error: $_poll_err"
            return
        }
        if {$sock eq ""} return
        if {$state ne "running"} return

        # Get current location
        set depth [info frame]
        if {[catch {set finfo [select_debug_location_frame]}]} return
        if {$finfo eq ""} return

        set file ""
        set line 0
        if {[dict exists $finfo file]} { set file [dict get $finfo file] }
        if {[dict exists $finfo line]} { set line [dict get $finfo line] }

        # Skip frames from agent internals
        if {[dict exists $finfo proc]} {
            set pname [dict get $finfo proc]
            if {[string match "::tcld_agent::*" $pname]} return
        }

        set breakpoint_hit 0
        if {$file ne "" && $line > 0 && [check_breakpoint $file $line]} {
            set breakpoint_hit 1
        }

        set trace_decision_ready 0
        set trace_decision_pause 0
        set trace_decision_reason ""

        send_msg "trace_point" [json_dict [list \
            file [json_string $file] \
            line [json_int $line] \
            frameCount [json_int [expr {$depth - 1}]] \
            level [json_int $depth] \
            breakpointHit [json_bool $breakpoint_hit] \
        ]]

        wait_for_trace_decision

        if {!$trace_decision_pause} return

        # Pause execution — suppress traces while sending stopped event
        set suppress_trace 1
        set state "paused"
        set paused_depth $depth
        if {$trace_decision_reason eq ""} {
            set trace_decision_reason "step"
        }

        send_msg "stopped" [json_dict [list \
            reason [json_string $trace_decision_reason] \
            file [json_string $file] \
            line [json_int $line] \
            frameCount [json_int [expr {$depth - 1}]] \
        ]]

        # Enter command loop — blocks until bridge sends continue/step
        # suppress_trace is managed by handle_command within the loop;
        # reset it here so poll_commands_nonblocking in the loop body
        # doesn't accidentally suppress. The command_loop itself doesn't
        # trigger user-visible traces.
        set suppress_trace 0
        command_loop
    }

    # ── Command loop (while paused) ───────────────────────────────────
    proc command_loop {} {
        variable sock
        variable state
        variable read_buf

        _debug_log "command_loop: entering, state=$state"

        while {$state eq "paused" && $sock ne ""} {
            # Read one line from socket (blocking)
            if {[catch {
                if {[gets $sock line] < 0} {
                    if {[eof $sock]} {
                        _debug_log "command_loop: eof on socket"
                        teardown
                        return
                    }
                    # No data yet, use vwait-like approach
                    after 10
                    continue
                }
            } err]} {
                _debug_log "command_loop: read error: $err"
                teardown
                return
            }

            set line [string trim $line]
            if {$line eq ""} continue

            _debug_log "command_loop: received line: [string range $line 0 120]..."

            if {[catch {
                set msg [parse_json_object $line]
            } err]} {
                _debug_log "command_loop: JSON parse error: $err"
                continue
            }

            if {[catch {handle_command $msg} err]} {
                _debug_log "command_loop: handle_command error: $err"
                catch {send_msg "output" [json_dict [list \
                    stream [json_string "stderr"] \
                    text [json_string "agent: handle_command error: $err"] \
                ]]}
            }

            _debug_log "command_loop: after handle, state=$state"
        }
        _debug_log "command_loop: exiting, state=$state"
    }

    # ── Handle a single command from bridge ───────────────────────────
    proc handle_command {msg} {
        variable state
        variable step_mode
        variable step_depth
        variable paused_depth
        variable breakpoints
        variable control_mode
        variable suppress_trace
        variable trace_decision_ready
        variable trace_decision_pause
        variable trace_decision_reason

        set suppress_trace 1
        set _handle_code [catch {
            set type [dict get $msg type]
            set data {}
            if {[dict exists $msg data]} {
                set data [dict get $msg data]
            }

            _debug_log "handle_command: type=$type, state=$state, paused_depth=$paused_depth"

            switch $type {
                trace_decision {
                    if {![dict exists $data pause]} {
                        break
                    }
                    set trace_decision_pause [dict get $data pause]
                    set trace_decision_reason ""
                    if {[dict exists $data reason]} {
                        set trace_decision_reason [dict get $data reason]
                    }
                    set trace_decision_ready 1
                }
                step_instruction {
                    catch {send_msg "output" [json_dict [list \
                        stream [json_string "stderr"] \
                        text [json_string "instruction stepping is only supported by embedded debugger runtime"] \
                    ]]}
                }
                step_into {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring step_into while state=$state"
                        break
                    }
                    set control_mode "step_into"
                    set step_mode "step_into"
                    set step_depth $paused_depth
                    set state "running"
                    _debug_log "handle_command: step_into → sending resumed"
                    send_msg "resumed" [json_dict [list controlMode [json_string "step_into"]]]
                }
                step_over {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring step_over while state=$state"
                        break
                    }
                    set control_mode "step_over"
                    set step_mode "step_over"
                    set step_depth $paused_depth
                    set state "running"
                    send_msg "resumed" [json_dict [list controlMode [json_string "step_over"]]]
                }
                step_out {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring step_out while state=$state"
                        break
                    }
                    set control_mode "step_out"
                    set step_mode "step_out"
                    set step_depth $paused_depth
                    set state "running"
                    send_msg "resumed" [json_dict [list controlMode [json_string "step_out"]]]
                }
                continue {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring continue while state=$state"
                        break
                    }
                    set control_mode "continue"
                    set step_mode ""
                    set state "running"
                    send_msg "resumed" [json_dict [list controlMode [json_string "continue"]]]
                }
                pause {
                    if {$state ne "running"} {
                        _debug_log "handle_command: ignoring pause while state=$state"
                        break
                    }
                    set control_mode "pause"
                    set step_mode "step_into"
                    set step_depth 999999
                }
                set_breakpoint {
                    set file [dict get $data file]
                    set file [normalize_file_path $file]
                    set line [dict get $data line]
                    set condition ""
                    if {[dict exists $data condition]} {
                        set condition [dict get $data condition]
                    }
                    set new_bps {}
                    foreach bp $breakpoints {
                        if {[dict get $bp file] ne $file || [dict get $bp line] != $line} {
                            lappend new_bps $bp
                        }
                    }
                    lappend new_bps [dict create file $file line $line condition $condition enabled 1]
                    set breakpoints $new_bps
                    send_msg "breakpoint_ack" [json_dict [list \
                        file [json_string $file] \
                        line [json_int $line] \
                        set [json_bool 1] \
                        enabled [json_bool 1] \
                    ]]
                }
                set_breakpoint_enabled {
                    set file [dict get $data file]
                    set file [normalize_file_path $file]
                    set line [dict get $data line]
                    set enabled [expr {[dict get $data enabled] ? 1 : 0}]
                    set found 0
                    set new_bps {}
                    foreach bp $breakpoints {
                        if {[dict get $bp file] eq $file && [dict get $bp line] == $line} {
                            dict set bp enabled $enabled
                            set found 1
                        }
                        lappend new_bps $bp
                    }
                    if {!$found} {
                        send_msg "command_error" [json_dict [list \
                            command [json_string "set_breakpoint_enabled"] \
                            controlMode [json_string $control_mode] \
                            code [json_int 404] \
                            message [json_string "breakpoint not found"] \
                            argc [json_int 3] \
                        ]]
                    } else {
                        set breakpoints $new_bps
                        send_msg "breakpoint_ack" [json_dict [list \
                            file [json_string $file] \
                            line [json_int $line] \
                            set [json_bool 1] \
                            enabled [json_bool $enabled] \
                        ]]
                    }
                }
                clear_breakpoint {
                    set file [dict get $data file]
                    set file [normalize_file_path $file]
                    set line [dict get $data line]
                    set new_bps {}
                    foreach bp $breakpoints {
                        if {[dict get $bp file] ne $file || [dict get $bp line] != $line} {
                            lappend new_bps $bp
                        }
                    }
                    set breakpoints $new_bps
                    send_msg "breakpoint_ack" [json_dict [list \
                        file [json_string $file] \
                        line [json_int $line] \
                        set [json_bool 0] \
                        enabled [json_bool 0] \
                    ]]
                }
                clear_all_breakpoints {
                    set count [llength $breakpoints]
                    set breakpoints {}
                    send_msg "breakpoints_cleared" [json_dict [list \
                        count [json_int $count] \
                    ]]
                }
                get_variables {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring get_variables while state=$state"
                        break
                    }
                    set request_id 0
                    if {[dict exists $data request_id]} {
                        set request_id [dict get $data request_id]
                    }
                    set level 1
                    if {[dict exists $data level]} {
                        set level [dict get $data level]
                    }
                    set vars [get_variables $level]
                    send_msg "variables" [json_dict [list \
                        requestId [json_int $request_id] \
                        level [json_int $level] \
                        vars $vars \
                    ]]
                }
                get_stack {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring get_stack while state=$state"
                        break
                    }
                    set request_id 0
                    if {[dict exists $data request_id]} {
                        set request_id [dict get $data request_id]
                    }
                    set stack [get_stack]
                    send_msg "stack" [json_dict [list \
                        requestId [json_int $request_id] \
                        frames $stack \
                    ]]
                }
                eval {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring eval while state=$state"
                        break
                    }
                    set request_id 0
                    if {[dict exists $data request_id]} {
                        set request_id [dict get $data request_id]
                    }
                    set script [dict get $data script]
                    set level 1
                    if {[dict exists $data level]} {
                        set level [dict get $data level]
                    }
                    set eval_json [eval_at_level $script $level]
                    set eval_json [string replace $eval_json 0 0 "\{[json_string "requestId"]:[json_int $request_id],"]
                    send_msg "eval_result" $eval_json
                }
                set_variable {
                    if {$state ne "paused"} {
                        _debug_log "handle_command: ignoring set_variable while state=$state"
                        break
                    }
                    set name [dict get $data name]
                    set value [dict get $data value]
                    set level 1
                    if {[dict exists $data level]} {
                        set level [dict get $data level]
                    }
                    if {[catch {
                        uplevel #$level [list set $name $value]
                    } err]} {
                        send_msg "eval_result" [json_dict [list \
                            result [json_string $err] \
                            error [json_bool 1] \
                        ]]
                    } else {
                        send_msg "eval_result" [json_dict [list \
                            result [json_string $value] \
                            error [json_bool 0] \
                        ]]
                    }
                }
            }
        } _handle_err]

        set suppress_trace 0

        if {$_handle_code} {
            _debug_log "handle_command: error: $_handle_err"
        }
    }

    # ── Stdout/stderr capture ─────────────────────────────────────────
    proc setup_output_capture {} {
        # Redirect puts to capture stdout/stderr
        rename ::puts ::tcld_agent::_original_puts

        proc ::puts {args} {
            set argc [llength $args]
            set nonewline 0
            set channel stdout
            set text ""

            if {$argc == 1} {
                set text [lindex $args 0]
            } elseif {$argc == 2} {
                if {[lindex $args 0] eq "-nonewline"} {
                    set nonewline 1
                    set text [lindex $args 1]
                } else {
                    set channel [lindex $args 0]
                    set text [lindex $args 1]
                }
            } elseif {$argc == 3} {
                set nonewline 1
                set channel [lindex $args 1]
                set text [lindex $args 2]
            }

            # Send to bridge if stdout or stderr
            if {$channel eq "stdout" || $channel eq "stderr"} {
                set stream [expr {$channel eq "stderr" ? "stderr" : "stdout"}]
                set output $text
                if {!$nonewline} {
                    append output "\n"
                }
                ::tcld_agent::send_msg "output" [::tcld_agent::json_dict [list \
                    stream [::tcld_agent::json_string $stream] \
                    text [::tcld_agent::json_string $output] \
                ]]
            }

            # Also write to actual channel
            if {$nonewline} {
                ::tcld_agent::_original_puts -nonewline $channel $text
            } else {
                ::tcld_agent::_original_puts $channel $text
            }
        }
    }

    # ── Socket accept callback ────────────────────────────────────────
    proc on_connect {chan addr port} {
        variable sock
        set sock $chan
        fconfigure $sock -buffering line -translation lf -encoding utf-8 -blocking 1

        # Set up output capture
        setup_output_capture

        # Now run the target script
        run_target
    }

    # ── Proc interception — auto-trace new procs ───────────────────
    proc setup_proc_tracing {} {
        foreach p [info procs ::*] {
            if {![string match "::tcld_agent::*" $p]} {
                add_execution_trace $p
            }
        }

        # Intercept `proc` so every new proc automatically gets an enterstep trace.
        rename ::proc ::tcld_agent::_original_proc

        ::tcld_agent::_original_proc ::proc {name arglist body} {
            # Create the proc normally
            ::tcld_agent::_original_proc $name $arglist $body

            # Resolve to fully qualified name
            set ns [uplevel 1 {namespace current}]
            if {[string match "::*" $name]} {
                set fqn $name
            } elseif {$ns eq "::"} {
                set fqn "::$name"
            } else {
                set fqn "${ns}::${name}"
            }

            # Don't trace our own procs
            if {[string match "::tcld_agent::*" $fqn]} return

            # Add enterstep trace so we can step into this proc
            catch { ::tcld_agent::add_execution_trace $fqn }
            # If this proc matches a configured hook command, wrap it now.
            catch { ::tcld_agent::ensure_command_hook_wrappers }
        }
    }

    proc setup_core_command_tracing {} {
        foreach command_name {
            source
            eval
            uplevel
            namespace
            interp
            after
            vwait
            update
        } {
            add_execution_trace $command_name
        }
    }

    # ── Teardown — restore all renamed commands and close socket ──────
    proc teardown {} {
        variable sock

        # Remove execution traces
        clear_execution_traces

        # Restore renamed commands
        if {[llength [info commands ::tcld_agent::_original_puts]] > 0} {
            catch { rename ::puts {} }
            catch { rename ::tcld_agent::_original_puts ::puts }
        }
        if {[llength [info commands ::tcld_agent::_original_proc]] > 0} {
            catch { rename ::proc {} }
            catch { rename ::tcld_agent::_original_proc ::proc }
        }
        if {[llength [info commands ::tcld_agent::_original_package]] > 0} {
            catch { rename ::package {} }
            catch { rename ::tcld_agent::_original_package ::package }
        }

        # Restore command hook wrappers
        variable command_hooks
        foreach cmd [dict keys $command_hooks] {
            set alias "::tcld_agent::__orig_[string map {: _} $cmd]"
            if {[llength [info commands $alias]] > 0} {
                catch { rename $cmd {} }
                catch { rename $alias $cmd }
            }
        }

        # Close socket
        if {$sock ne ""} {
            catch { close $sock }
            set sock ""
        }
    }

    # ── Run the target script ─────────────────────────────────────────
    proc run_target {} {
        variable target_script
        variable target_args
        variable state
        variable step_mode
        variable debug_extension_loaded
        variable debug_extension_installed
        variable debug_extension_capabilities
        variable debug_extension_reason

        set state "running"
        # Start paused (step_into mode so first command triggers pause)
        set step_mode "step_into"

        # Set argv for the target script
        set ::argv $target_args
        set ::argc [llength $target_args]
        set ::argv0 $target_script

        maybe_load_debug_extension

        # Send ready event only after extension setup is complete.
        send_msg "ready" [json_dict [list \
            tclVersion [json_string [info patchlevel]] \
            pid [json_int [pid]] \
            debugExtensionLoaded [json_bool $debug_extension_loaded] \
            debugExtensionInstalled [json_bool $debug_extension_installed] \
            debugExtensionCapabilities [json_string $debug_extension_capabilities] \
            debugExtensionReason [json_string $debug_extension_reason] \
        ]]

        # Install tracing: top-level source + auto-trace new procs
        setup_core_command_tracing
        setup_proc_tracing
        setup_bytecode_stream_hooks

        if {[catch {
            source $target_script
        } err opts]} {
            set error_info ""
            if {[dict exists $opts -errorinfo]} {
                set error_info [dict get $opts -errorinfo]
            }
            send_msg "stopped" [json_dict [list \
                reason [json_string "exception"] \
                file [json_string $target_script] \
                line [json_int 0] \
                frameCount [json_int 0] \
                error [json_string $err] \
                errorInfo [json_string $error_info] \
            ]]

            # Enter command loop so bridge can inspect after exception
            set state "paused"
            command_loop
        }

        # Clean up
        clear_execution_traces

        # Send exit event
        send_msg "exited" [json_dict [list \
            code [json_int 0] \
        ]]

        variable sock
        if {$sock ne ""} {
            catch {close $sock}
        }
    }

    # ── Main entry point ──────────────────────────────────────────────
    proc main {argv} {
        variable target_script
        variable target_args
        variable port
        variable bytecode_hook_mode
        variable bytecode_hook_commands
        variable debug_extension_path

        set bytecode_hook_commands {}
        if {[catch {ensure_bytecode_hook_support_loaded} err]} {
            puts stderr "tcld-agent: $err"
            exit 1
        }
        ensure_debug_extension_support_loaded

        # Parse arguments
        set i 0
        while {$i < [llength $argv]} {
            set arg [lindex $argv $i]
            switch -- $arg {
                --port {
                    incr i
                    set port [lindex $argv $i]
                }
                --bytecode-hook-mode {
                    incr i
                    set bytecode_hook_mode [lindex $argv $i]
                }
                --bytecode-hook-command {
                    incr i
                    lappend bytecode_hook_commands [lindex $argv $i]
                }
                --debug-extension-path {
                    incr i
                    set debug_extension_path [lindex $argv $i]
                }
                --script {
                    incr i
                    set target_script [lindex $argv $i]
                    # Everything after --script <path> is target args
                    set target_args [lrange $argv [expr {$i + 1}] end]
                    break
                }
                default {
                    puts stderr "tcld-agent: unknown argument: $arg"
                    exit 1
                }
            }
            incr i
        }

        if {[catch {set_bytecode_hook_mode $bytecode_hook_mode} err]} {
            puts stderr "tcld-agent: $err"
            exit 1
        }
        set bytecode_hook_commands [normalize_bytecode_hook_commands $bytecode_hook_commands]

        if {$port == 0 || $target_script eq ""} {
            puts stderr "Usage: tclsh tcld-agent.tcl --port <port> \[--bytecode-hook-mode passthrough|intercept\] \[--bytecode-hook-command <command>\]... \[--debug-extension-path <path>\] --script <target.tcl> \[args...\]"
            exit 1
        }

        if {![file exists $target_script]} {
            puts stderr "tcld-agent: script not found: $target_script"
            exit 1
        }
        set target_script [normalize_file_path $target_script]

        # Open TCP server socket and wait for bridge to connect
        set server [socket -server ::tcld_agent::on_connect $port]

        # Enter event loop (on_connect will be called when bridge connects)
        vwait ::tcld_agent::sock
    }
}

# Run
::tcld_agent::main $argv
