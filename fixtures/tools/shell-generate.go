// Run from repos/sdk: go run ../../fixtures/tools/shell-generate.go > ../../fixtures/tools/shell.json
package main

import (
 "encoding/json"
 "os"
 "github.com/gratefulagents/sdk/pkg/agentsdk/policy"
 "github.com/gratefulagents/sdk/pkg/agentsdk/tools/shell"
)
func main() {
 commands := []string{
 "echo safe", "echo gh", "git status", "git remote -v", "git commit -m local", "git push origin feature", "git push origin main", "git\tpush origin HEAD:main", "/usr/bin/git -C repo push origin +feature:refs/heads/master", "command git push origin master:release", "env -u GIT_ASKPASS git push origin main", "sudo -u deploy git push origin HEAD:master", "nice -n 5 git push origin main", "git;push origin main", "bash -c 'git push origin main'", "printf 'git push origin main' | bash", "gh pr merge 5 --squash", "bash -c 'gh pr create --fill'", "echo body | gh issue create --title t", "env -S 'gh pr list'", "env -u GITHUB_TOKEN gh pr merge 5", "sudo -u deploy gh pr merge 5", "nice -n 10 gh api /user", "timeout 30 gh pr checks 5", "env -u GITHUB_TOKEN ls", "timeout 30 make test", "X=1 git push origin main", "echo $HOME", "wc -l $(find . -name '*.go')", "echo `ls`", "printf $'hello'", "cat <<< hello", "cat <(ls)", "eval ls", "source file", ". file", "alias foo=ls", "foo() { ls; }", "rm -fr /", "sudo rm -r /*", "chmod -R /", "chown -R /", "dd if=x of=/dev/sda", "mkfs.ext4 /tmp/foo", "tee /etc/hosts", "echo x >/etc/hosts", "ls 2>/dev/null", "echo x >/dev/stderr", "echo x | tee /dev/stdout", "git add file", "git reset --hard", "git checkout feature", "git fetch origin", "echo '$(literal)'", "echo '$HOME'", "echo \\$HOME",
 }
 cases := []map[string]any{}
 for _, mode := range []policy.PermissionMode{policy.PermissionModeReadOnly,policy.PermissionModeWorkspaceWrite,policy.PermissionModeDangerFullAccess} {
  for _, command := range commands {
   blocked, reason := shell.IsCommandBlockedForMode(mode,command)
   cases = append(cases,map[string]any{"mode":mode,"command":command,"blocked":blocked,"reason":reason})
  }
 }
 schemas := []map[string]any{}
 for _, env := range []map[string]string{{},{"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS":"999","GRATEFUL_BASH_MAX_TIMEOUT_MS":"500","GRATEFUL_BASH_MAX_OUTPUT_BYTES":"999999999"},{"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS":"240000","GRATEFUL_BASH_MAX_TIMEOUT_MS":"120000"},{"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS":"bad","GRATEFUL_BASH_MAX_TIMEOUT_MS":"-1"}} {
  for _, key := range []string{"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS","GRATEFUL_BASH_MAX_TIMEOUT_MS","GRATEFUL_BASH_MAX_OUTPUT_BYTES"} { os.Unsetenv(key) }
  for key,value := range env { os.Setenv(key,value) }
  bash,start := &shell.BashTool{},&shell.BashStartTool{}
  schemas=append(schemas,map[string]any{"environment":env,"Bash":json.RawMessage(bash.InputSchema()),"BashStart":json.RawMessage(start.InputSchema()),"bash_description":bash.Description(),"start_description":start.Description()})
 }
 enc:=json.NewEncoder(os.Stdout); enc.SetIndent("","  "); if err:=enc.Encode(map[string]any{"policy":cases,"schemas":schemas}); err!=nil {panic(err)}
}
