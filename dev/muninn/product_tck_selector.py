"""Deterministic selection of product TCK profiles."""


def select_profiles(requested, profiles):
    """Return requested profiles in canonical order, or all profiles by default."""
    if requested is None:
        return tuple(profiles)

    if len(requested) != len(set(requested)):
        raise ValueError('No se permiten perfiles duplicados')

    unknown = [profile for profile in requested if profile not in profiles]
    if unknown:
        raise ValueError(f'Perfil desconocido: {", ".join(unknown)}')

    requested_set = set(requested)
    return tuple(profile for profile in profiles if profile in requested_set)
