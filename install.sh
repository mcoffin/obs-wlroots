#!/bin/bash
set -e
set -x

build_type=${1:-release}
target_dir="$(pwd)/target"
plugin_name=obs_wlroots

mkpd() {
	mkdir -p $1
	pushd $_
}

mkpd ~/.config/obs-studio/plugins/$plugin_name/bin/64bit
ln -s -f $target_dir/$build_type/libobs_wlroots.so libobs_wlroots.so
popd
