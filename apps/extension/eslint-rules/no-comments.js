const DIRECTIVE =
  /^\s*(?:eslint(?:-disable|-enable|-env)?\b|global\b|globals\b|exported\b|@ts-(?:check|nocheck|expect-error)\b|@vitest-environment\b|prettier-ignore\b|\/\s*<reference\b)/;

module.exports = {
  meta: {
    type: 'suggestion',
    docs: {
      description:
        'Disallow comments: the why goes to docs/decisions, the what to names and tests.',
    },
    messages: {
      noComment:
        'Comment not allowed: delete it, move the decision to docs/decisions, or file an issue (/delete-comment).',
    },
    schema: [],
  },
  create(context) {
    return {
      Program() {
        for (const comment of context.sourceCode.getAllComments()) {
          if (comment.type === 'Shebang' || DIRECTIVE.test(comment.value)) continue;
          context.report({ loc: comment.loc, messageId: 'noComment' });
        }
      },
    };
  },
};
