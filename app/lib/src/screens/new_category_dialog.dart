import 'package:flutter/material.dart';

import '../api.dart';

typedef GroupLoader = Future<List<CategoryGroupDto>> Function();
typedef GroupCreator = Future<CategoryGroupDto> Function(String name);
typedef CategoryCreator = Future<CategoryDto> Function({
  required int groupId,
  required String name,
  required bool taxable,
});

/// What each group is for, in words rather than in a name.
///
/// The names are the seed's and the CLI shows them, so they stay — but
/// "Non-Monthly" describes a schedule and not a purpose, and the purpose is
/// what someone is choosing between. Saying it plainly here costs nothing and
/// is the difference between picking a group and guessing at one.
const Map<String, String> _groupBlurbs = {
  'Bills': 'Paid on a schedule, and the amount is what you pay — rent, '
      'insurance, a car payment.',
  'Frequent': 'Day-to-day shopping, where a till adds sales tax.',
  'Occasional': 'Money set aside each month for something that comes round now '
      'and then — repairs, travel, gifts. You still record what goes out when '
      'it does.',
  'Quality of Life': 'What the money is for, once the necessities are covered.',
  'Education': 'Tuition, lessons, books. Ships empty, because education is a '
      'different set of costs for everyone.',
  'Savings': 'Money put aside rather than spent.',
};

/// Add a category.
class NewCategoryDialog extends StatefulWidget {
  const NewCategoryDialog({
    super.key,
    this.loadGroups = listCategoryGroups,
    this.create = createCategory,
    this.createGroup = createCategoryGroup,
  });

  final GroupLoader loadGroups;
  final CategoryCreator create;
  final GroupCreator createGroup;

  @override
  State<NewCategoryDialog> createState() => _NewCategoryDialogState();
}

class _NewCategoryDialogState extends State<NewCategoryDialog> {
  final TextEditingController _name = TextEditingController();
  List<CategoryGroupDto> _groups = const [];
  int? _groupId;

  /// Whether a till adds sales tax to this.
  ///
  /// No default that could be wrong quietly: taxed is the column's default and
  /// is right for most shopping, but a car payment taxed at 7.45% is $537.25
  /// where the statement says $500.00.
  bool _taxed = true;

  bool _saving = false;
  String? _problem;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final groups = await widget.loadGroups();
      if (mounted) setState(() => _groups = groups);
    } catch (error) {
      if (mounted) setState(() => _problem = 'Could not load the groups: $error');
    }
  }

  /// Make a group, and select it.
  Future<void> _newGroup() async {
    final name = await showDialog<String>(
      context: context,
      builder: (context) => const _NameAGroup(),
    );
    if (name == null || name.trim().isEmpty || !mounted) return;

    try {
      final created = await widget.createGroup(name.trim());
      if (!mounted) return;
      setState(() {
        _groups = [..._groups, created];
        _groupId = created.id;
        _problem = null;
      });
    } catch (error) {
      if (mounted) setState(() => _problem = '$error');
    }
  }

  Future<void> _save() async {
    final name = _name.text.trim();
    final groupId = _groupId;
    if (name.isEmpty) {
      setState(() => _problem = 'Give the category a name.');
      return;
    }
    if (groupId == null) {
      setState(() => _problem = 'Pick where it belongs.');
      return;
    }

    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      final created = await widget.create(
        groupId: groupId,
        name: name,
        taxable: _taxed,
      );
      if (mounted) Navigator.of(context).pop(created);
    } catch (error) {
      if (mounted) {
        setState(() {
          _problem = '$error';
          _saving = false;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final String? blurb =
        _groupId == null ? null : _groupBlurbs[_nameOfGroup(_groupId!)];

    return AlertDialog(
      title: const Text('New category'),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(
                controller: _name,
                autofocus: true,
                textCapitalization: TextCapitalization.words,
                decoration: const InputDecoration(
                  labelText: 'Name',
                  hintText: 'Car Payment',
                  border: OutlineInputBorder(),
                ),
              ),
              const SizedBox(height: 16),
              Row(
                children: [
                  Expanded(
                    child: DropdownButtonFormField<int>(
                      initialValue: _groupId,
                      isExpanded: true,
                      decoration: const InputDecoration(
                        labelText: 'Where it belongs',
                        border: OutlineInputBorder(),
                      ),
                      hint: const Text('Choose a group…'),
                      items: [
                        for (final group in _groups)
                          DropdownMenuItem<int>(
                            value: group.id,
                            child: Text(group.name),
                          ),
                      ],
                      onChanged: (id) => setState(() => _groupId = id),
                    ),
                  ),
                  const SizedBox(width: 8),
                  // The seeded groups are one person's idea of how spending
                  // divides up. Somebody whose life has a shape the list does
                  // not should not have to accept the nearest fit.
                  IconButton.filledTonal(
                    onPressed: _newGroup,
                    icon: const Icon(Icons.create_new_folder_outlined),
                    tooltip: 'New group',
                  ),
                ],
              ),
              if (blurb != null)
                Padding(
                  padding: const EdgeInsets.only(top: 8),
                  child: Text(
                    blurb,
                    style: theme.textTheme.bodySmall
                        ?.copyWith(color: theme.colorScheme.onSurfaceVariant),
                  ),
                ),
              const SizedBox(height: 20),
              Text('Sales tax', style: theme.textTheme.labelLarge),
              const SizedBox(height: 4),
              RadioGroup<bool>(
                groupValue: _taxed,
                onChanged: (value) => setState(() => _taxed = value ?? true),
                child: const Column(
                  children: [
                    RadioListTile<bool>(
                      value: true,
                      contentPadding: EdgeInsets.zero,
                      title: Text('Added at the till'),
                      subtitle: Text('You enter what it rang up before tax.'),
                    ),
                    RadioListTile<bool>(
                      value: false,
                      contentPadding: EdgeInsets.zero,
                      title: Text('Already in the amount'),
                      subtitle: Text(
                        'A bill or a payment: what you enter is what you paid.',
                      ),
                    ),
                  ],
                ),
              ),
              if (_problem != null) ...[
                const SizedBox(height: 12),
                Text(
                  _problem!,
                  style: theme.textTheme.bodyMedium
                      ?.copyWith(color: theme.colorScheme.error),
                ),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: _saving ? null : () => _save(),
          child: const Text('Add category'),
        ),
      ],
    );
  }

  String _nameOfGroup(int id) =>
      _groups.firstWhere((g) => g.id == id).name;
}

/// Asking for one string.
class _NameAGroup extends StatefulWidget {
  const _NameAGroup();

  @override
  State<_NameAGroup> createState() => _NameAGroupState();
}

class _NameAGroupState extends State<_NameAGroup> {
  final TextEditingController _name = TextEditingController();

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('New group'),
      content: TextField(
        controller: _name,
        autofocus: true,
        textCapitalization: TextCapitalization.words,
        onSubmitted: (value) => Navigator.of(context).pop(value),
        decoration: const InputDecoration(
          labelText: 'Name',
          hintText: 'Education',
          border: OutlineInputBorder(),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_name.text),
          child: const Text('Add group'),
        ),
      ],
    );
  }
}
