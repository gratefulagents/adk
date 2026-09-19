"""Generate inputs for git-generate.go; expected results come only from the pinned SDK."""
import json

cases = []
def add(name, tool, input=None, **kwargs):
    cases.append(dict(name=name, tool=tool, input={} if input is None else input, **kwargs))
def reply(output='', error=''):
    return dict(output=output, error=error)
pr='create_pull_request'
issue='create_github_issue'
attach='attach_repository'
view='pr view --json url -q .url'
branch='rev-parse --abbrev-ref HEAD'
default='symbolic-ref --short refs/remotes/origin/HEAD'
url='https://github.com/acme/repo/pull/7\n'
issueurl='https://github.com/acme/repo/issues/3\n'
add('pr-dirty-title-draft',pr,dict(title='Add feature',base_branch='main',draft=True),git={'status --porcelain':reply(' M changed\n?? new\n')},gh={view:reply(url)})
add('pr-untracked-default-message',pr,git={'status --porcelain':reply('?? new\n')},gh={view:reply(url)})
add('pr-existing',pr,gh={'pr create --head agent/work --fill':reply('create failed','already exists'),view:reply(url)})
add('pr-fallback',pr,gh={'pr create --head agent/work --fill':reply(url),view:reply('','exit status 1')})
add('pr-unvalidated-fallback-sdk',pr,gh={'pr create --head agent/work --fill':reply('not a url'),view:reply('not https')})
add('pr-body-only',pr,{'body':'hello <world> & friends\u2028\u2029'},gh={view:reply(url)})
add('pr-title-body',pr,{'title':'x','body':'details'},gh={view:reply(url)})
add('pr-empty-output',pr)
add('pr-create-failure',pr,gh={'pr create --head agent/work --fill':reply('bad','exit status 1')})
for value in ['main','master','HEAD','','develop','release']:
    add('pr-guard-'+value,pr,{'base_branch':' release '},git={branch:reply(value+'\n'),default:reply('origin/develop\n')})
add('pr-guard-error',pr,git={branch:reply('fatal','not a repository')})
add('pr-default-unavailable',pr,git={default:reply('','not set')},gh={view:reply(url)})
add('pr-status-failure-still-pushes',pr,git={'status --porcelain':reply(' M file','exit status 1')},gh={view:reply(url)})
for command in ['add -A','commit --no-verify -m changes from agent run','push --no-verify -u origin HEAD']:
    add('pr-error-'+command,pr,git={'status --porcelain':reply(' M file'),command:reply('stderr','exit status 1')})
add('pr-subrepo',pr,{'repo_path':'repos/repo'},setup='subrepo',gh={view:reply(url)})
add('pr-missing-repo',pr,{'repo_path':'nope'})
add('pr-outside-repo',pr,{'repo_path':'../outside'})
for inp in [{},{'title':'  '},{'title':3},{'draft':'yes'},['bad']]:
    add('invalid-'+str(inp),pr if 'draft' in inp or isinstance(inp,list) else issue,inp)
add('null-fields',pr,{'title':None,'draft':None},gh={view:reply(url)})
add('case-insensitive',issue,{'TITLE':'Bug'},gh={'issue create --title Bug':reply(issueurl)})
add('issue-simple',issue,{'title':'Bug','body':'Details','assignees':['octo']},gh={'issue create --title Bug --body Details --assignee octo':reply(issueurl)})
labels='label list --limit 1000 --json name'
add('issue-normalize-labels',issue,{'title':'Bug','labels':[' tests ','sdk','TESTS','']},gh={labels:reply('[]'),'label create --color BFD4F2 -- sdk':reply('label already exists\n','exit status 1'),'issue create --title Bug --label tests --label sdk':reply(issueurl)})
add('issue-existing-labels',issue,{'title':'Bug','labels':['bug','SDK']},gh={labels:reply('[{"name":"Bug"},{"name":"sdk"}]'),'issue create --title Bug --label bug --label SDK':reply(issueurl)})
add('issue-dash-label',issue,{'title':'Bug','labels':['--help']},gh={labels:reply('[]'),'issue create --title Bug --label --help':reply(issueurl)})
for output,error in [('bad',''),('',''),('stderr','exit status 1')]:
    add('issue-output-'+output,issue,{'title':'Bug'},gh={'issue create --title Bug':reply(output,error)})
add('issue-label-list-error',issue,{'title':'Bug','labels':['bug']},gh={labels:reply('stderr','exit status 1')})
add('issue-label-create-error',issue,{'title':'Bug','labels':['bug']},gh={labels:reply('[]'),'label create --color BFD4F2 -- bug':reply('denied','exit status 1')})
for output in ['null','[{"name":null},null,{}]','not json','{}','[3]','[{"name":3}]']:
    add('issue-label-list-'+output,issue,{'title':'Bug','labels':['bug']},gh={labels:reply(output),'issue create --title Bug --label bug':reply(issueurl)})
add('issue-subrepo',issue,{'title':'Bug','repo_path':'repos/repo'},setup='subrepo',gh={'issue create --title Bug':reply(issueurl)})
add('attach-defaults',attach,{'repository':'acme/repo'},base=' main ',branch=' agent/work ',setup='root_git')
add('attach-repo-alias',attach,{'repo':'github.com/Acme/Repo.git','alias':' My strange REPO.git '})
add('attach-custom-store',attach,{'repository':'acme/repo'},store='clones',setup='root_git')
add('attach-root-store',attach,{'repository':'acme/repo'},store='.',setup='root_git')
add('attach-empty-alias',attach,{'repository':'acme/repo','alias':'...___---'})
add('attach-long-alias',attach,{'repository':'acme/repo','alias':'a'*90})
add('attach-fallback',attach,{'repository':'acme/repo','base_branch':'missing','branch_name':'work'},git={'clone missing':reply('fatal: Remote branch missing not found in upstream origin','exit status 128')})
add('attach-fallback-fails',attach,{'repository':'acme/repo','base_branch':'missing'},git={'clone missing':reply('Remote branch missing not found in upstream','exit status 128'),'clone':reply('denied','exit status 128')})
for error in ['signal: killed','timeout','context deadline exceeded','context canceled','exit status 128']:
    add('attach-clone-'+error,attach,{'repository':'acme/repo','base_branch':'main'},git={'clone main':reply('Cloning into dest\n',error)})
add('attach-checkout-error',attach,{'repository':'acme/repo','branch_name':'bad'},git={'checkout -B bad':reply('bad branch','exit status 1')})
for setup in ['existing','plain','incomplete','working']:
    git={'remote get-url origin':reply('https://github.com/acme/repo.git\n')}
    if setup in ['incomplete','working']:git['rev-parse --verify --quiet HEAD^{commit}']=reply('','exit status 1')
    add('attach-'+setup,attach,{'repository':'acme/repo','branch_name':'ignored'},setup=setup,git=git)
for origin in ['git@github.com:acme/repo.git','https://github.com/acme/other.git','https://user@github.com/acme/repo.git']:
    add('attach-origin-'+origin,attach,{'repository':'acme/repo'},setup='existing',git={'remote get-url origin':reply(origin)})
add('attach-origin-failed',attach,{'repository':'acme/repo'},setup='existing',git={'remote get-url origin':reply('stderr','exit status 1')})
for repository in ['', '../repo','-uploader','git@github.com:acme/repo.git','ssh://git@github.com/acme/repo.git','git://github.com/acme/repo','file:///tmp/repo','ext::helper','https://user@github.com/acme/repo','https://github.com:443/acme/repo','https://github.com/acme/repo?q=1','https://github.com/acme/repo#frag','https://github.com.evil.test/acme/repo','https://github.com/acme/repo/extra','https://github.com/acme/%72epo','https://github.com/acme/../repo','acme/re po','HTTPS://GITHUB.COM/acme/repo','https://github.com/acme/repo?','https://github.com/acme/repo#']:
    add('attach-url-'+repository,attach,{'repository':repository})
for output in ['[', '{bad}', '[{"name":}]', '[{"name":"bug",}]', '[{"name":"bug"} false]', '[] trailing', 'true', '"string"', '3', '[false]', '[{"NAME":"Bug"}]']:
    add('issue-label-edge-'+output,issue,{'title':'Bug','labels':['bug']},gh={labels:reply(output),'issue create --title Bug --label bug':reply(issueurl)})
for repository in ['https://github.com/acme/re%20po','https://github.com/acme/re%3Fpo','https://github.com/acme/re%5Cpo', 'https://github.com/acme/re\\po','https://github.com/acme/re%7Fpo','https://github.com/acme/re%ffpo','https://github.com/acme/re%41po', 'https://github.com/acme/re%00po']:
    add('attach-url-edge-'+repository,attach,{'repository':repository})
for inp in [{'title':True},{'title':[]},{'title':{}},{'title':'Bug','labels':3},{'title':'Bug','labels':[3]}, {'repository':3}, {'repository':'acme/repo','alias':False}]:
    add('invalid-edge-'+str(inp),attach if 'repository' in inp else issue,inp)
add('attach-unicode-alias',attach,{'repository':'acme/repo','alias':'İfoo ΟΣ bar'})
add('issue-unicode-labels',issue,{'title':'Bug','labels':['İfoo','ifoo','ΟΣ','Οσ']},gh={labels:reply('[]'),'issue create --title Bug --label İfoo --label ΟΣ':reply(issueurl)})
add('unicode-case-folding-keys',attach,{'repository':'acme/repo','baſe_branch':'main'})
with open('fixtures/tools/git-cases.json','w') as f:
    json.dump(cases,f,indent=2);f.write('\n')
