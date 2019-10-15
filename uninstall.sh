#!/bin/bash
set -e
set -x

uninstall_obs_plugin() {
	pushd ~/.config/obs-studio/plugins
	if [ -d $1 ]; then
		rm -rf $1
	fi
	popd
}

uninstall_obs_plugin obs_wlroots
