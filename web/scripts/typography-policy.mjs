import postcss from 'postcss';
import ts from 'typescript';

// rem units follow the user's browser font preference. The minimum is 1rem.
export function readableSize(value) {
  value = value.trim();
  if (value === 'inherit' || value === 'var(--text-body)') return true;
  if (/^(?:\d*\.)?\d+rem$/.test(value)) return parseFloat(value) >= 1;
  const clamp = /^clamp\(\s*([\d.]+)rem\s*,\s*[\d.]+vw\s*,\s*([\d.]+)rem\s*\)$/.exec(value);
  return !!clamp && Number(clamp[1]) >= 1 && Number(clamp[2]) >= Number(clamp[1]);
}

export function checkCss(source, file) {
  const errors = [];
  postcss.parse(source, { from: file }).walkDecls(declaration => {
    const property = declaration.prop.toLowerCase();
    if ((property === 'font-size' && !readableSize(declaration.value))
      || (property === 'font' && declaration.value !== 'inherit')
      || (property === '--text-body' && declaration.value.trim() !== '1rem')) {
      errors.push(`${file}:${declaration.source.start.line}: ${property}: ${declaration.value} — use at least 1rem or var(--text-body)`);
    }
  });
  return errors;
}

export function checkScript(source, file) {
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const errors = [];
  const check = (node, value) => {
    if (value && ts.isStringLiteral(value) && readableSize(value.text)) return;
    const { line } = tree.getLineAndCharacterOfPosition(node.getStart(tree));
    errors.push(`${file}:${line + 1}: fontSize must be a literal readable rem size; dynamic sizing belongs in CSS`);
  };
  const visit = node => {
    if (ts.isPropertyAssignment(node)) {
      const property = node.name.getText(tree).replace(/['"]/g, '');
      if (['fontSize', 'font-size'].includes(property)) check(node, node.initializer);
      if (property === 'font' && (!ts.isStringLiteral(node.initializer) || node.initializer.text !== 'inherit')) {
        const { line } = tree.getLineAndCharacterOfPosition(node.getStart(tree));
        errors.push(`${file}:${line + 1}: font shorthand must be inherit; declare fontSize explicitly`);
      }
    }
    if (ts.isJsxAttribute(node) && ['fontSize', 'font-size'].includes(node.name.getText(tree))) {
      const value = node.initializer;
      check(node, value && ts.isJsxExpression(value) ? value.expression : value);
    }
    ts.forEachChild(node, visit);
  };
  visit(tree);
  return errors;
}
