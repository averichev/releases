.PHONY: release release-plan

release:
	sh scripts/release.sh

release-plan:
	sh scripts/release.sh --plan
