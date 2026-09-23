import { mkdir, writeFile, chmod } from 'node:fs/promises';
import path from 'node:path';

export async function createFixtures(root) {
  const applications = path.join(root, 'data/applications');
  const system = path.join(root, 'system/applications');
  const directory = path.join(root, 'program with spaces');
  await mkdir(directory, { recursive: true });
  const program = path.join(directory, 'record arguments');
  const record = path.join(root, 'launched.jsonl');
  await writeFile(program, '#!/usr/bin/python3\nimport json, os, sys\nwith open(os.environ["MAESTRIA_LAUNCHER_ARGV_RECORD"], "a", encoding="utf-8") as output:\n    output.write(json.dumps(sys.argv[1:], ensure_ascii=False) + "\\n")\n');
  await chmod(program, 0o755);
  const entry = (name, extra = '', exec = `"${program}"`) => `[Desktop Entry]\nType=Application\nName=${name}\nComment=Isolated launcher acceptance fixture\nExec=${exec}\nTerminal=false\nKeywords=CatalogProbe;fixture;\n${extra}\n`;
  const launchName = 'Fixture Ω 日本語 — an intentionally long application title for accessible ellipsis';
  const launchPath = path.join(applications, 'maestria-test-launch.desktop');
  const exec = `"${program}" "two words" "\\\\$HOME" "literal;not-a-shell" %c %k %% %f %u`;
  await writeFile(launchPath, entry(launchName, '', exec));
  await writeFile(path.join(system, 'maestria-test-override.desktop'), entry('System Shadowed Fixture'));
  await writeFile(path.join(applications, 'maestria-test-override.desktop'), entry('User Priority Fixture'));
  await writeFile(path.join(system, 'maestria-test-hidden.desktop'), entry('System Hidden Fixture'));
  await writeFile(path.join(applications, 'maestria-test-hidden.desktop'), entry('Hidden Fixture', 'Hidden=true'));
  await writeFile(path.join(applications, 'maestria-test-nodisplay.desktop'), entry('NoDisplay Fixture', 'NoDisplay=true'));
  await writeFile(path.join(applications, 'maestria-test-onlyshow.desktop'), entry('Wrong Desktop Fixture', 'OnlyShowIn=KDE;'));
  await writeFile(path.join(applications, 'maestria-test-notshow.desktop'), entry('Excluded Desktop Fixture', 'NotShowIn=GNOME;'));
  await writeFile(path.join(applications, 'maestria-test-tryexec.desktop'), entry('Unavailable TryExec Fixture', 'TryExec=/maestria-test-missing-program'));
  const removedPath = path.join(applications, 'maestria-test-removed.desktop');
  await writeFile(removedPath, entry('Removable Fixture'));
  const selectedFile = path.join(root, 'selected local file.txt');
  await writeFile(selectedFile, 'A local file chosen explicitly; never indexed.\n');
  return { applications, program, record, launchName, launchPath, removedPath, selectedFile };
}
