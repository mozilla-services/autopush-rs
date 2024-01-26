DB_DSN=dual \
    DB_SETTINGS="{\"primary\":{\"db_settings\":\"{\\\"message_family\\\":\\\"message\\\",\\\"router_family\\\":\\\"router\\\",\\\"table_name\\\":\\\"projects/test/instances/test/tables/test\\\"}\",\"dsn\":\"grpc://bigtable.googleapis.com\"},\"secondary\":{\"db_settings\":\"{\\\"message_table\\\":\\\"test_message\\\",\\\"router_table\\\":\\\"test_router\\\"}\",\"dsn\":\"http://localhost:8000/\"}}" \
    TEST_RESULTS_DIR=tmp \
    make integration-test
