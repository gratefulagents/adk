package sandbox

import (
	"encoding/hex"
	"os"
	"strings"
	"testing"
	"unicode"
)

func nativeHexMap(values map[string]string) map[string]string {
	result := map[string]string{}
	for key, value := range values {
		result[hex.EncodeToString([]byte(key))] = hex.EncodeToString([]byte(value))
	}
	return result
}

func nativeHexList(values []string) []string {
	result := make([]string, 0, len(values))
	for _, value := range values {
		result = append(result, hex.EncodeToString([]byte(value)))
	}
	return result
}

func nativeEnvironmentCases(t *testing.T) []map[string]any {
	original := os.Environ()
	defer func() {
		os.Clearenv()
		for _, pair := range original {
			key, value, _ := strings.Cut(pair, "=")
			if err := os.Setenv(key, value); err != nil {
				t.Fatal(err)
			}
		}
	}()
	cases := []map[string]any{}
	record := func(parent map[string]string, config Config) {
		os.Clearenv()
		for key, value := range parent {
			if err := os.Setenv(key, value); err != nil {
				t.Fatalf("fixture environment key %s: %v", key, err)
			}
		}
		cases = append(cases, map[string]any{
			"parent": nativeHexMap(parent),
			"config": map[string]any{
				"path":    hex.EncodeToString([]byte(config.Path)),
				"prepend": nativeHexList(config.PathPrepend),
				"append":  nativeHexList(config.PathAppend),
				"goRoot":  hex.EncodeToString([]byte(config.GOROOT)),
				"extra":   nativeHexMap(config.ExtraEnv),
			},
			"output": nativeHexMap(SafeEnvMapWithConfig(config)),
		})
	}
	configs := []Config{
		{Path: "/custom/bin:relative:/usr/bin:/custom/bin", GOROOT: "/custom/go", ExtraEnv: map[string]string{"JAVA_HOME": "/opt/jdk", "TOOL_PATH": "$PATH:/extra,with-comma", "BAD-NAME": "no"}},
		{},
		{Path: "/fixed:relative:/fixed:/b", PathPrepend: []string{"/ignored"}, PathAppend: []string{"/ignored-too"}},
		{Path: "relative", PathPrepend: []string{"/first", "/first/./", "relative", "/a:/b"}, PathAppend: []string{"/last", "/first"}},
		{Path: "/../.././:/a//b/:/a/b", GOROOT: "\u00a0/custom/../go/ \u3000"},
		{GOROOT: "relative", ExtraEnv: map[string]string{"A": "$PATH", "B": "$A:$OPENAI_API_KEY", " C ": " padded ", "1BAD": "no", "ZERO": "x\x00y"}},
		{ExtraEnv: map[string]string{" A ": "first", "A": "second", " B ": "$A", "B": "$B:$A", "bad-name": "no", "LC_new": ""}},
		{ExtraEnv: map[string]string{"PATH": "$PATH:/extra", "GOROOT": "relative", "HOME": "/chosen", "Z": "$HOME:$GOROOT:$PATH"}},
		{Path: "/nul\x00path", GOROOT: "/go\xff", ExtraEnv: map[string]string{"BINARY": "\xff$PATH", "BAD\xff": "no"}},
		{PathPrepend: []string{"/a/../../b", "//a///b", "\u00a0/x\u3000"}, PathAppend: []string{"/", "/../"}},
		{ExtraEnv: map[string]string{"": "no", "\u3000": "no", "_": "ok", "A1": "$_", "Z": "${MISSING}:$?"}},
	}
	for _, path := range []string{"", "relative", "/usr/bin:/bin:/usr/bin", " /a/../b : : . :/ ", "/.././:/a//b/:/a/b", "/x\xff:/x", "\u00a0"} {
		parent := map[string]string{"PATH": path, "HOME": "/home/dummy", "USER": "test", "LOGNAME": "test", "SHELL": "/bin/sh", "TERM": "dumb", "COLORTERM": "truecolor", "LANG": " C ", "TZ": "UTC", "TMPDIR": "/tmp/dummy", "LC_": "", "LC_COLLATE": " C ", "LC_BAD-NAME": "no", "OPENAI_API_KEY": "dummy-secret", "GOROOT": "/ambient-go", "OTHER": "no"}
		for _, config := range configs {
			record(parent, config)
		}
	}
	for r := rune(0); r <= unicode.MaxRune; r++ {
		if unicode.IsSpace(r) {
			boundary := string(r)
			record(map[string]string{"LANG": boundary, "LC_TEST": boundary}, Config{Path: boundary + "/a/../b" + boundary, GOROOT: boundary + "/go/" + boundary, ExtraEnv: map[string]string{boundary + "A" + boundary: boundary + "value" + boundary}})
		}
	}
	for _, boundary := range []string{"\u180e", "\ufeff", "\u200b", "\xff"} {
		record(map[string]string{"LANG": boundary}, Config{Path: boundary + "/path", ExtraEnv: map[string]string{boundary + "A": "no", "VALUE": boundary}})
	}
	return cases
}

func nativeConfigCases(t *testing.T) []map[string]any {
	original := os.Environ()
	defer func() {
		os.Clearenv()
		for _, pair := range original {
			key, value, _ := strings.Cut(pair, "=")
			if err := os.Setenv(key, value); err != nil {
				t.Fatal(err)
			}
		}
	}()
	cases := []map[string]any{}
	record := func(parent map[string]string) {
		os.Clearenv()
		for key, value := range parent {
			if err := os.Setenv(key, value); err != nil {
				t.Fatalf("fixture key %s: %v", key, err)
			}
		}
		config := ConfigFromEnv()
		cases = append(cases, map[string]any{
			"parent": nativeHexMap(parent),
			"config": map[string]any{
				"mode":                     hex.EncodeToString([]byte(config.Mode)),
				"runningInKubernetes":      config.RunningInKubernetes,
				"workspaceRoot":            hex.EncodeToString([]byte(config.WorkspaceRoot)),
				"path":                     hex.EncodeToString([]byte(config.Path)),
				"prepend":                  nativeHexList(config.PathPrepend),
				"append":                   nativeHexList(config.PathAppend),
				"goRoot":                   hex.EncodeToString([]byte(config.GOROOT)),
				"extra":                    nativeHexMap(config.ExtraEnv),
				"readOnlyPaths":            nativeHexList(config.ExtraReadOnlyPaths),
				"writablePaths":            nativeHexList(config.ExtraWritablePaths),
				"exposeServiceAccount":     config.ExposeKubernetesServiceAccount,
				"allowUnsafeReadOnlyLocal": config.AllowUnsafeReadOnlyLocal,
			},
			"output": nativeHexMap(SafeEnvMapWithConfig(config)),
		})
	}
	flags := []string{"", "1", "0", "t", "T", "true", "TRUE", "True", "TrUe", "f", "F", "false", "FALSE", "False", "FaLsE", "yes", "YeS", "y", "Y", "on", "ON", "oN", "off", "2", "enabled", " \ttrue\n", " \u00a0yes\u3000", "trueish", "猫", "\xff"}
	extras := []string{"", " \u3000", "A=1,B=$A", " A = one , A=two,1BAD=x,BAD-NAME=x", "A=a,b\nB=b,c\n", "A=a\rB=b", "missing,=empty,B=valid", "A==x", "A=$OPENAI_API_KEY", "A=\xff,BAD\xff=x", "\u00a0A\u3000=one,A=two", "OPENAI_API_KEY=explicit-dummy"}
	paths := []string{"", "relative", "/usr/bin:/bin:/usr/bin", " /a/../b : : . :/ ", "/.././:/a//b/:/a/b", "/x\xff:/x", "\u00a0"}
	for i, flag := range flags {
		for j, extra := range extras {
			record(map[string]string{
				SandboxModeEnv:                           " required \n",
				SandboxPathEnv:                           paths[(i+j)%len(paths)],
				SandboxPathPrependEnv:                    "relative:/a:/a/./::/b/..",
				SandboxPathAppendEnv:                     "/last:/a",
				SandboxExtraReadOnlyPathsEnv:             "/one:/one/.:relative:/../two",
				SandboxExtraWritablePathsEnv:             "/scratch/../scratch:/scratch",
				SandboxExtraEnvEnv:                       extra,
				SandboxExposeKubernetesServiceAccountEnv: flag,
				SandboxAllowUnsafeReadOnlyLocalEnv:       flags[(i+j)%len(flags)],
				"GOROOT":                                 " /go/../root ",
				"PATH":                                   "/usr/bin:/bin",
				"OPENAI_API_KEY":                         "ambient-dummy",
				"KUBERNETES_SERVICE_HOST":                "ignored",
				"GRATEFULAGENTS_WORKSPACE_ROOT":          "/ignored",
			})
		}
	}
	record(map[string]string{
		SandboxModeEnv:                           "required",
		SandboxPathEnv:                           "/custom/bin:relative:/usr/bin",
		SandboxPathPrependEnv:                    "/prepend/bin:relative",
		SandboxPathAppendEnv:                     "/append/bin",
		SandboxExtraReadOnlyPathsEnv:             "/opt/tooling:relative",
		SandboxExtraWritablePathsEnv:             "/tmp/scratch:relative",
		SandboxExtraEnvEnv:                       "JAVA_HOME=/opt/jdk\nTOOL_PATH=$PATH:/tooling\nBAD-NAME=no",
		SandboxExposeKubernetesServiceAccountEnv: "true",
	})
	record(map[string]string{SandboxAllowUnsafeReadOnlyLocalEnv: "1"})
	record(nil)
	return cases
}
