if __name__ == '__main__':
    import argparse
    import dev
    parser = argparse.ArgumentParser(
        description='development tool for bluemetal',
    )

    args = parser.parse_args()
    exit(dev.main())
