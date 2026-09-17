// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk. Executes actual pinned runtime bundle construction, not a
// reimplementation of its feature/access selection rules.
package main
import (
 "context"
 "encoding/json"
 "fmt"
 "os"
 "reflect"
 "sort"
 "strings"
 "github.com/gratefulagents/sdk/pkg/agentsdk/policy"
 runtime "github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
)
type entry struct {Name string `json:"name"`;Feature string `json:"feature"`;ReadOnly bool `json:"read_only"`}
type row struct {Features *[]string `json:"features"`;Legacy uint8 `json:"legacy"`;Access string `json:"access"`;Remote bool `json:"remote"`;Private bool `json:"private"`;Allowed []string `json:"allowed"`;Names []string `json:"names"`}
func must(err error){if err!=nil{panic(err)}}
func main(){
 data,err:=os.ReadFile("../../crates/adk-tools/src/manifest.json");must(err);var entries []entry;must(json.Unmarshal(data,&entries));seen:=map[string]bool{};mutating:=map[string]bool{};for _,e:=range entries {seen[e.Feature]=true;if !e.ReadOnly{mutating[e.Name]=true}}
 features:=[]string{};allow:=[]string{};for k:=range seen{features=append(features,k)};for k:=range mutating{allow=append(allow,k)};sort.Strings(features);sort.Strings(allow)
 work,err:=os.MkdirTemp("","adk-registry-");must(err);defer os.RemoveAll(work)
 rows:=[]row{}
 add:=func(r row){
  cfg:=runtime.Config{WorkDir:work,ProjectStateDir:work+"/state",ProjectID:"registry-fixture",AllowedMutatingTools:r.Allowed,AllowPrivateNetworkURLs:r.Private}
  switch r.Access {case "read_only":cfg.PermissionMode=policy.PermissionModeReadOnly;case "workspace_write":cfg.PermissionMode=policy.PermissionModeWorkspaceWrite;case "full_access":cfg.PermissionMode=policy.PermissionModeDangerFullAccess}
  if r.Remote {cfg.GitRemoteWrites=policy.GitRemoteWritesEnabled}else{cfg.GitRemoteWrites=policy.GitRemoteWritesDisabled}
  if r.Features!=nil {cfg.Features=&runtime.Features{};for _,f:=range *r.Features {var v reflect.Value;if strings.HasPrefix(f,"ProjectState."){v=reflect.ValueOf(&cfg.Features.ProjectState).Elem();f=strings.TrimPrefix(f,"ProjectState.")}else{v=reflect.ValueOf(&cfg.Features.Tools).Elem()};for _,part:=range strings.Split(f,"."){v=v.FieldByName(part)};if !v.IsValid(){panic(f)};v.SetBool(true)}} else {cfg.EnableTools=r.Legacy&1!=0;cfg.EnableSubAgents=r.Legacy&2!=0;cfg.DisableDefaultTools=r.Legacy&4!=0;cfg.DisableSignalTools=r.Legacy&8!=0;cfg.DisableWebTools=r.Legacy&16!=0;cfg.EnableAsyncShell=r.Legacy&32!=0;cfg.EnableProjectState=r.Legacy&64!=0}
  bundle,err:=runtime.BuildToolBundle(context.Background(),cfg);must(err);r.Names=[]string{};for _,tool:=range bundle.Tools {r.Names=append(r.Names,tool.Name())};sort.Strings(r.Names);for _,closer:=range bundle.Closers{must(closer.Close())};rows=append(rows,r)
 }
 for _,access:=range []string{"read_only","workspace_write","full_access"}{for flags:=0;flags<4;flags++{for _,allowed:=range [][]string{{},allow}{base:=row{Access:access,Remote:flags&1!=0,Private:flags&2!=0,Allowed:allowed};empty:=[]string{};base.Features=&empty;add(base);base.Features=&features;add(base);for _,f:=range features{one:=[]string{f};base.Features=&one;add(base)};base.Features=nil;for legacy:=0;legacy<128;legacy++{base.Legacy=uint8(legacy);add(base)}}}}
 encoded,err:=json.Marshal(rows);must(err);must(os.WriteFile("../../fixtures/tools/registry-matrix.json",append(encoded,'\n'),0644));fmt.Printf("%d actual SDK registry/bundle mode cases generated\n",len(rows))
}
