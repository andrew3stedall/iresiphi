const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const api = require('..');
const golden = JSON.parse(fs.readFileSync('../../tests/fixtures/two_suppliers.parity.json', 'utf8'));
test('recipe load, save, hash, validate, explain and owned close', () => {
  const recipe = new api.Recipe(golden.canonical_json);
  assert.equal(recipe.hash, golden.recipe_hash);
  assert.equal(recipe.canonicalJson(), golden.canonical_json);
  assert.equal(new api.Recipe(recipe.toJson()).hash, recipe.hash);
  assert.equal(recipe.validate().valid, true);
  assert.ok(recipe.explain());
  recipe.close(); recipe.close();
  assert.throws(() => recipe.hash, {code: 'HANDLE_CLOSED'});
});
