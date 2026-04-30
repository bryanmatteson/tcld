namespace eval ::tcld_debug_extension {
    proc load_and_install {path} {
        set path [string trim $path]
        if {$path eq ""} {
            return [dict create loaded 0 installed 0 reason "empty_path"]
        }
        if {![file exists $path]} {
            return [dict create loaded 0 installed 0 reason "missing_file"]
        }

        if {[catch {load $path Tcld_dbg} err]} {
            return [dict create loaded 0 installed 0 reason "load_failed" error $err]
        }

        set installed 0
        set install_error ""
        if {[llength [info commands ::tcld::dbg::install]] > 0} {
            if {[catch {::tcld::dbg::install} install_err]} {
                set install_error $install_err
            } else {
                set installed 1
            }
        }

        set caps ""
        if {[llength [info commands ::tcld::dbg::capabilities]] > 0} {
            catch {set caps [::tcld::dbg::capabilities]}
        }

        return [dict create \
            loaded 1 \
            installed $installed \
            capabilities $caps \
            installError $install_error \
            reason "ok"]
    }
}
