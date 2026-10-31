#!/usr/bin/env bash
set -euo pipefail

root="${CI_PRIMARY_REPOSITORY_PATH:-$(cd "$(dirname "$0")/.." && pwd)}"
python3 - "$root" <<'PY'
import importlib.util,json,os,pathlib,sys,time
root=pathlib.Path(sys.argv[1])
spec=importlib.util.spec_from_file_location("native_cache",root/"scripts/mobile-native-cache.py")
helper=importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
start=root/"build/testflight/archive-start.json"
if start.is_file():
    elapsed=max(0,time.time()-json.loads(start.read_text())["started"])
    helper.record_timing(root,"native-archive",elapsed,int(os.environ.get("CI_XCODEBUILD_EXIT_CODE","1")))
receipt=root/"build/testflight/cloud-timings.json"
if receipt.is_file():
    result=json.loads(receipt.read_text())
    result["snapshot_sha"]=os.environ.get("CI_COMMIT","")
    result["cloud_build_url"]=os.environ.get("CI_BUILD_URL","")
    print("PENNEL_TESTFLIGHT_TIMINGS="+json.dumps(result,sort_keys=True))
PY
