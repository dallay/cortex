"""Fail-closed validation of one pytest-json-report profile result."""


def validate_profile_result(expected_nodeids, process_exit_code, report):
    """Reject an empty, partial, duplicated, failed, or inconsistent profile.

    Args:
        expected_nodeids: Exact node ID set selected for this profile.
        process_exit_code: Exit status returned by the pytest subprocess.
        report: Parsed pytest-json-report document.

    Raises:
        ValueError: If the process or report does not prove complete success.
    """
    expected = set(expected_nodeids)
    if not expected:
        raise ValueError('empty_expected_profile')
    if process_exit_code != 0:
        raise ValueError('pytest_process_failed')
    if type(report) is not dict or report.get('exitcode') != 0:
        raise ValueError('pytest_report_failed')

    tests = report.get('tests')
    if type(tests) is not list or len(tests) != len(expected):
        raise ValueError('pytest_result_count_mismatch')

    observed = []
    for test in tests:
        if type(test) is not dict or type(test.get('nodeid')) is not str:
            raise ValueError('pytest_result_invalid')
        nodeid = test['nodeid']
        observed.append(nodeid)
        if test.get('outcome') != 'passed':
            raise ValueError('pytest_test_not_passed')
        for phase in ('setup', 'call', 'teardown'):
            value = test.get(phase)
            if type(value) is not dict or value.get('outcome') != 'passed':
                raise ValueError('pytest_phase_not_passed')

    if len(set(observed)) != len(observed):
        raise ValueError('duplicate_pytest_nodeid')
    if set(observed) != expected:
        raise ValueError('pytest_nodeids_mismatch')

    summary = report.get('summary')
    if type(summary) is not dict:
        raise ValueError('pytest_summary_missing')
    if summary.get('passed') != len(expected) or summary.get('total') != len(expected):
        raise ValueError('pytest_summary_count_mismatch')
    for outcome in ('failed', 'skipped', 'error', 'xfailed', 'xpassed'):
        if summary.get(outcome, 0) != 0:
            raise ValueError('pytest_summary_not_clean')
