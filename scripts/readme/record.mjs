/** Capture the actual native Tcld, with the public fixture and Default layout. */
import {mkdir, writeFile} from 'node:fs/promises';
import {homedir} from 'node:os';
import {join} from 'node:path';

export async function record(app, output = join(homedir(), 'Documents/Developer/tcld/readme/frames')) {
  await mkdir(output, {recursive:true});
  const frames = [];
  async function state() { return app.getAXState({emit:false, disableDiffing:true}); }
  async function capture(file, caption, duration = 4) {
    await state();
    await writeFile(join(output, file), await app.getScreenshot({emit:false}));
    frames.push({file, caption, duration});
  }
  // These tab headers are not separate accessibility controls. Use coordinates
  // from the real window screenshot; normalized against its current pixel size.
  async function tabAt(x) {
    const image = Buffer.from(await app.getScreenshot({emit:false}));
    let width, height;
    if (image.subarray(1,4).toString() === 'PNG') {
      width = image.readUInt32BE(16); height = image.readUInt32BE(20);
    } else if (image.readUInt16BE(0) === 0xffd8) {
      // CUA currently returns baseline JPEG; walk header segments to its SOF.
      for (let offset = 2; offset + 9 < image.length;) {
        if (image[offset] !== 0xff) throw new Error('Invalid JPEG header');
        const marker = image[offset+1], length = image.readUInt16BE(offset+2);
        if ([0xc0,0xc1,0xc2].includes(marker)) {
          height = image.readUInt16BE(offset+5); width = image.readUInt16BE(offset+7); break;
        }
        if (length < 2) throw new Error('Invalid JPEG segment');
        offset += length + 2;
      }
    }
    if (!width || !height) throw new Error('Unrecognized screenshot dimensions');
    await app.click([Math.round(width*x), Math.round(height*204/1800)]);
    await state();
  }
  const initial = await state();
  if (!initial.includes('public_complex_fixture.tbc') || !initial.includes('LAYOUT Default')) {
    throw new Error('Open the public_complex_fixture.tbc sample and select Default before recording.');
  }
  await tabAt(170/2800); // Disassembly
  await app.scroll([400,600], "up", 10);
  const listing = await state();
  const first = listing.match(/^\s*(\d+) text c000 .*0x0000 push1 0 /m);
  if (!first) throw new Error("First instruction is not visible after scrolling to the top");
  await app.click(Number(first[1]));
  if (!(await state()).includes("pc 0x0000")) throw new Error("First instruction was not selected");
  await capture('workbench.jpg', 'Disassembly, recovered Tcl, and one synchronized inspector', 5);
  const current = await state();
  const instruction = current.match(/^\s*(\d+) text c004 0x0032 push1 17 /m);
  if (!instruction) throw new Error('Expected the public fixture instruction for logLevelInit');
  await app.click(Number(instruction[1]));
  const linked = await state();
  if (!linked.includes('pc 0x0032')) throw new Error('Source navigation did not follow the selected instruction');
  await capture('follow-source.jpg', 'Select an instruction and follow its source and stack effect', 5);
  await tabAt(500/2800); // Symbols
  const symbols = await state();
  const procedure = symbols.match(/^\s*(\d+) button PROC fixture::logLevelInit .*$/m);
  if (!procedure) throw new Error('Expected the recovered logLevelInit symbol');
  await app.click(Number(procedure[1]));
  await capture('symbols.jpg', 'Navigate recovered procedures, namespaces, and variables', 5);
  await tabAt(1330/2800); // Compare
  const compared = await state();
  if (!compared.includes('source ↔ disasm')) throw new Error('Compare panel did not open');
  await capture('comparison.jpg', 'Inspect recovered commands and source-to-bytecode mappings', 5);
  await tabAt(1000/2800); // Return to Source View
  await writeFile(join(output, 'frames.json'), JSON.stringify(frames, null, 2)+'\n');
  return {output, frames:frames.length};
}
